// Check if product_id is valid
pub fn is_valid_product_id(product_id: &str) -> bool {
    product_id.chars().all(|c| c.is_alphanumeric() || c == '_')
}

// Check if field is valid
pub fn is_valid_field(field: &str) -> bool {
    matches!(field, "sellPrice" | "buyPrice" | "sellVolume" | "buyVolume" | "sellOrders" | "buyOrders" | "sellMovingWeek" | "buyMovingWeek")
}