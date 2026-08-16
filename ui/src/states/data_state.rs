use std::fmt::Debug;

#[derive(Debug, Clone)]
pub enum DataState<T: Debug + Clone> {
    None,
    Loading,
    Loaded(T),
    Error(String),
}

impl<T: Debug + Clone> DataState<T> {
    pub fn as_ref(&self) -> &Self {
        self
    }
}

impl<T: Debug + Clone> From<T> for DataState<T> {
    fn from(value: T) -> Self {
        Self::Loaded(value)
    }
}
