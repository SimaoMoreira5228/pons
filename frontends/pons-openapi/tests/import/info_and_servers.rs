use super::document;

#[test]
fn imports_info_servers_and_extensions() {
    let document = document();
    assert_eq!(document.info.title, "Orders");
    assert_eq!(document.info.version, "1.0.0");
    assert_eq!(
        document.info.description.as_deref(),
        Some("Order management API.")
    );
    assert_eq!(
        document.info.servers,
        ["https://api.example.test", "https://sandbox.example.test"]
    );
    assert_eq!(document.info.ext["openapi.x.internal/v1"]["team"], "sales");
    assert_eq!(document.ext["openapi.x.audience/v1"], "public");
}
