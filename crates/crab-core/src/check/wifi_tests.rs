use super::wifi_text_error;

#[test]
fn password_rules() {
    assert!(wifi_text_error("password", "").is_none());
    assert!(wifi_text_error("password", "12345678").is_none());
    assert!(wifi_text_error("password", "short").is_some());
    assert!(wifi_text_error("ap_password", &"a".repeat(64)).is_none());
    assert!(wifi_text_error("ap_password", &"g".repeat(64)).is_some());
    assert!(wifi_text_error("password", "парольпароль").is_some());
    assert!(wifi_text_error("ssid", &"a".repeat(33)).is_some());
    assert!(wifi_text_error("ssid", "Сеть").is_none());
}
