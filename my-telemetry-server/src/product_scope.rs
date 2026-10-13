use std::sync::Arc;

/// Where the metrics go whose writer names no product scope - and what a reader which names
/// none is given
pub const DEFAULT_PRODUCT_SCOPE: &str = "default";

/// The metadata of gRPC `Upload` and the header of `POST /api/add` naming the product scope
/// of the metrics: the name my-logger-server reads it by. A writer takes it from `product`
/// of its connection string.
pub const SCOPE_HEADER: &str = "scope";

/// A scope names a folder: a-z, 0-9, '-' and '_'
pub fn is_valid_scope_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
}

/// The name as a scope is named - trimmed, in lower case - if it can name one
pub fn to_scope_name(name: &str) -> Option<String> {
    let name = name.trim().to_lowercase();
    is_valid_scope_name(&name).then_some(name)
}

/// The scope the metrics of a writer go to: the one it names, the default one when it names
/// none - or one which can not name a folder.
pub fn scope_of_writer(named: Option<&str>) -> Arc<str> {
    match named.map(|named| (named, to_scope_name(named))) {
        Some((_, Some(scope))) => scope.into(),
        Some((named, None)) => {
            // An empty header is a writer with no product, not a mistake of one
            if !named.trim().is_empty() {
                println!(
                    "Product scope '{}' can not name a scope - a name is a-z, 0-9, '-' and '_', 64 characters at most: the metrics go to '{}'",
                    named, DEFAULT_PRODUCT_SCOPE
                );
            }

            DEFAULT_PRODUCT_SCOPE.into()
        }
        None => DEFAULT_PRODUCT_SCOPE.into(),
    }
}

/// The scope a reader asks for: the default one when it names none. An error for a name
/// which can not name a scope - it would be read as a path.
pub fn scope_of_reader(named: Option<&str>) -> Result<Arc<str>, String> {
    match named.map(str::trim) {
        None | Some("") => Ok(DEFAULT_PRODUCT_SCOPE.into()),
        Some(named) => to_scope_name(named).map(Into::into).ok_or_else(|| {
            format!(
                "'{}' is not a product scope: a name is a-z, 0-9, '-' and '_'",
                named
            )
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_writer_with_no_scope_writes_to_the_default_one() {
        assert_eq!(&*scope_of_writer(None), DEFAULT_PRODUCT_SCOPE);
        assert_eq!(&*scope_of_writer(Some("")), DEFAULT_PRODUCT_SCOPE);
        assert_eq!(&*scope_of_writer(Some(" My-Product_1 ")), "my-product_1");
        // Can not name a folder
        assert_eq!(&*scope_of_writer(Some("../etc")), DEFAULT_PRODUCT_SCOPE);
        assert_eq!(&*scope_of_writer(Some("my product")), DEFAULT_PRODUCT_SCOPE);
        assert_eq!(&*scope_of_writer(Some(&"a".repeat(65))), DEFAULT_PRODUCT_SCOPE);
    }

    #[test]
    fn a_reader_is_refused_a_name_which_is_not_a_scope() {
        assert_eq!(&*scope_of_reader(None).unwrap(), DEFAULT_PRODUCT_SCOPE);
        assert_eq!(&*scope_of_reader(Some(" ")).unwrap(), DEFAULT_PRODUCT_SCOPE);
        assert_eq!(&*scope_of_reader(Some(" Trading ")).unwrap(), "trading");
        assert!(scope_of_reader(Some("../etc")).is_err());
    }
}
