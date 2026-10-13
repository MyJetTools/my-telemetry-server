use std::fmt;

pub struct RequestError {
    pub message: String,
}

impl fmt::Display for RequestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl From<flurl::FlUrlError> for RequestError {
    fn from(err: flurl::FlUrlError) -> Self {
        Self {
            message: format!("{:?}", err),
        }
    }
}

impl From<my_http_utils::my_json::json_reader::JsonParseError> for RequestError {
    fn from(err: my_http_utils::my_json::json_reader::JsonParseError) -> Self {
        Self {
            message: err.into_string(),
        }
    }
}
