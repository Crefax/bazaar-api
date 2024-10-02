pub fn is_valid_product_id(product_id: &str) -> bool {
    product_id.chars().all(|c| c.is_alphanumeric() || c == '_')
}

pub fn is_valid_field(field: &str) -> bool {
    matches!(field, "sellPrice" | "buyPrice" | "sellVolume" | "buyVolume" | "sellOrders" | "buyOrders" | "sellMovingWeek" | "buyMovingWeek")
}