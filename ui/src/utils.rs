use rust_extensions::base64::*;

// The action name travels inside a URL PATH segment (`/last/:service/:action`), so
// the standard base64 alphabet will not do: its `/` would split the segment in two
// and the router would never match the route back. Encoding with the url-safe
// alphabet (RFC 4648 §5) keeps the round-trip intact for any action name.

pub fn to_base_64(src: &str) -> String {
    let encoded = src.as_bytes().into_base64();
    encoded.replace('+', "-").replace('/', "_")
}

pub fn from_base_64(src: &str) -> String {
    let restored = src.replace('-', "+").replace('_', "/");

    let Ok(bytes) = restored.from_base64() else {
        return String::new();
    };

    String::from_utf8(bytes).unwrap_or_default()
}
