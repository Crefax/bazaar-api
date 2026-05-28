use crate::db;
use crate::models::ProblemResponse;
use crate::state::{AppState, RateLimitOutcome};
use actix_web::http::StatusCode;
use actix_web::http::header::{HeaderName, HeaderValue};
use actix_web::{HttpRequest, HttpResponse};
use chrono::Utc;
use sha2::{Digest, Sha256};
use std::time::Duration;

pub const USER_API_KEY_HEADER: &str = "X-API-Key";
pub const ADMIN_API_KEY_HEADER: &str = "X-Admin-Api-Key";
pub const ADMIN_SESSION_COOKIE: &str = "bazaar_admin_session";

#[derive(Debug, Clone)]
pub struct RequestAuth {
    pub rate_limit: RateLimitOutcome,
}

pub fn hash_api_key(key: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(key.as_bytes());
    hex::encode(hasher.finalize())
}

pub fn generate_user_api_key() -> (String, String, String) {
    let prefix = uuid::Uuid::new_v4()
        .simple()
        .to_string()
        .chars()
        .take(10)
        .collect::<String>();
    let secret = format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    );
    let key = format!("bzusr_{}_{}", prefix, secret);
    let hash = hash_api_key(&key);
    (key, prefix, hash)
}

pub fn parse_api_key_prefix(key: &str) -> Option<&str> {
    let mut parts = key.split('_');
    match (parts.next(), parts.next(), parts.next()) {
        (Some("bzusr"), Some(prefix), Some(_)) if !prefix.is_empty() => Some(prefix),
        _ => None,
    }
}

pub fn constant_time_eq(left: &str, right: &str) -> bool {
    if left.len() != right.len() {
        return false;
    }

    left.bytes()
        .zip(right.bytes())
        .fold(0u8, |acc, (a, b)| acc | (a ^ b))
        == 0
}

pub fn problem(status: StatusCode, code: &str, message: &str) -> HttpResponse {
    HttpResponse::build(status).json(ProblemResponse::new(code, message, status.as_u16()))
}

pub fn with_rate_limit_headers(
    mut response: HttpResponse,
    rate_limit: &RateLimitOutcome,
) -> HttpResponse {
    if let Ok(value) = HeaderValue::from_str(&rate_limit.limit.to_string()) {
        response
            .headers_mut()
            .insert(HeaderName::from_static("x-ratelimit-limit"), value);
    }
    if let Ok(value) = HeaderValue::from_str(&rate_limit.remaining.to_string()) {
        response
            .headers_mut()
            .insert(HeaderName::from_static("x-ratelimit-remaining"), value);
    }
    if !rate_limit.allowed {
        if let Ok(value) = HeaderValue::from_str(&rate_limit.retry_after_seconds.to_string()) {
            response
                .headers_mut()
                .insert(HeaderName::from_static("retry-after"), value);
        }
    }
    response
}

pub async fn authorize_public(
    req: &HttpRequest,
    state: &AppState,
    required_scope: &str,
) -> Result<RequestAuth, HttpResponse> {
    let policy = db::get_access_policy(&state.db).await.map_err(|_| {
        problem(
            StatusCode::INTERNAL_SERVER_ERROR,
            "access_policy_error",
            "Access policy could not be loaded",
        )
    })?;

    if let Some(raw_key) = header_value(req, USER_API_KEY_HEADER) {
        if state
            .config
            .admin_api_key
            .as_deref()
            .is_some_and(|admin_key| constant_time_eq(admin_key, raw_key))
        {
            return Err(problem(
                StatusCode::FORBIDDEN,
                "admin_key_not_allowed",
                "Admin key cannot be used as a public API key",
            ));
        }

        let prefix = parse_api_key_prefix(raw_key).ok_or_else(|| {
            problem(
                StatusCode::UNAUTHORIZED,
                "invalid_api_key",
                "Invalid API key format",
            )
        })?;
        let record = db::find_api_key_by_prefix(&state.db, prefix)
            .await
            .map_err(|_| {
                problem(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "api_key_lookup_error",
                    "API key could not be verified",
                )
            })?
            .ok_or_else(|| {
                problem(
                    StatusCode::UNAUTHORIZED,
                    "invalid_api_key",
                    "Invalid API key",
                )
            })?;

        if !constant_time_eq(&record.key_hash, &hash_api_key(raw_key)) {
            return Err(problem(
                StatusCode::UNAUTHORIZED,
                "invalid_api_key",
                "Invalid API key",
            ));
        }
        if record.status != "active" {
            return Err(problem(
                StatusCode::FORBIDDEN,
                "api_key_disabled",
                "API key is not active",
            ));
        }
        if record
            .expires_at
            .is_some_and(|expires_at| expires_at <= Utc::now())
        {
            return Err(problem(
                StatusCode::FORBIDDEN,
                "api_key_expired",
                "API key has expired",
            ));
        }
        if !record.scopes.iter().any(|scope| scope == required_scope) {
            return Err(problem(
                StatusCode::FORBIDDEN,
                "scope_denied",
                "API key does not have the required scope",
            ));
        }

        let id = record.id.map(|id| id.to_hex()).unwrap_or_default();
        let rate_limit = state
            .rate_limiter
            .check(
                &format!("api-key:{}", id),
                record.rate_limit_per_minute,
                Duration::from_secs(60),
            )
            .await;

        if !rate_limit.allowed {
            return Err(with_rate_limit_headers(
                problem(
                    StatusCode::TOO_MANY_REQUESTS,
                    "rate_limited",
                    "Rate limit exceeded",
                ),
                &rate_limit,
            ));
        }

        let _ = db::touch_api_key_last_used(&state.db, &id).await;
        return Ok(RequestAuth { rate_limit });
    }

    if !policy.anonymous_public_enabled {
        return Err(problem(
            StatusCode::UNAUTHORIZED,
            "api_key_required",
            "Anonymous public API access is disabled",
        ));
    }

    let ip = request_ip(req, state);
    let rate_limit = state
        .rate_limiter
        .check(
            &format!("anonymous:{}", ip),
            policy.anonymous_rate_limit_per_minute,
            Duration::from_secs(60),
        )
        .await;

    if !rate_limit.allowed {
        return Err(with_rate_limit_headers(
            problem(
                StatusCode::TOO_MANY_REQUESTS,
                "rate_limited",
                "Rate limit exceeded",
            ),
            &rate_limit,
        ));
    }

    Ok(RequestAuth { rate_limit })
}

pub async fn authorize_admin(req: &HttpRequest, state: &AppState) -> Result<(), HttpResponse> {
    let configured_key = state.config.admin_api_key.as_deref().ok_or_else(|| {
        problem(
            StatusCode::SERVICE_UNAVAILABLE,
            "admin_auth_not_configured",
            "ADMIN_API_KEY is not configured",
        )
    })?;

    if let Some(raw_key) = header_value(req, ADMIN_API_KEY_HEADER) {
        if constant_time_eq(configured_key, raw_key) {
            return Ok(());
        }
        return Err(problem(
            StatusCode::FORBIDDEN,
            "invalid_admin_key",
            "Invalid admin API key",
        ));
    }

    if let Some(cookie) = req.cookie(ADMIN_SESSION_COOKIE) {
        if state.admin_sessions.verify(cookie.value()).await {
            return Ok(());
        }
    }

    Err(problem(
        StatusCode::UNAUTHORIZED,
        "admin_auth_required",
        "Admin authentication required",
    ))
}

pub fn verify_admin_key(raw_key: &str, state: &AppState) -> Result<(), HttpResponse> {
    let configured_key = state.config.admin_api_key.as_deref().ok_or_else(|| {
        problem(
            StatusCode::SERVICE_UNAVAILABLE,
            "admin_auth_not_configured",
            "ADMIN_API_KEY is not configured",
        )
    })?;

    if constant_time_eq(configured_key, raw_key) {
        Ok(())
    } else {
        Err(problem(
            StatusCode::FORBIDDEN,
            "invalid_admin_key",
            "Invalid admin API key",
        ))
    }
}

fn header_value<'a>(req: &'a HttpRequest, name: &str) -> Option<&'a str> {
    req.headers()
        .get(name)
        .and_then(|value| value.to_str().ok())
}

fn request_ip(req: &HttpRequest, state: &AppState) -> String {
    if state.config.trust_proxy {
        if let Some(forwarded_for) = header_value(req, "X-Forwarded-For") {
            if let Some(first_ip) = forwarded_for.split(',').next() {
                let ip = first_ip.trim();
                if !ip.is_empty() {
                    return ip.to_string();
                }
            }
        }
    }

    req.peer_addr()
        .map(|addr| addr.ip().to_string())
        .unwrap_or_else(|| "unknown".to_string())
}

#[cfg(test)]
mod tests {
    use super::{constant_time_eq, generate_user_api_key, hash_api_key, parse_api_key_prefix};

    #[test]
    fn generated_user_key_can_be_hashed_and_parsed() {
        let (key, prefix, hash) = generate_user_api_key();

        assert_eq!(parse_api_key_prefix(&key), Some(prefix.as_str()));
        assert!(constant_time_eq(&hash, &hash_api_key(&key)));
    }
}
