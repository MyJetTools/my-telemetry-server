use rust_extensions::{
    date_time::DateTimeAsMicroseconds,
    sorted_vec::{EntityWithStrKey, SortedVecWithStrKey},
};
use serde::*;

use crate::reader_grpc::PermanentUserStatusGrpcModel;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct PermanentUserPersistModel {
    pub user: String,
    pub created: i64,
    pub status: i32,
}

impl EntityWithStrKey for PermanentUserPersistModel {
    fn get_key(&self) -> &str {
        &self.user
    }
}

pub struct PermanentUsersList {
    pub users: SortedVecWithStrKey<PermanentUserPersistModel>,
}

impl PermanentUsersList {
    pub fn new() -> Self {
        Self {
            users: SortedVecWithStrKey::new(),
        }
    }

    pub fn is_permanent(&self, user_id: &str) -> bool {
        self.users.contains(user_id)
    }

    pub fn get_all(&self) -> Vec<PermanentUserPersistModel> {
        let mut result = Vec::new();
        for itm in self.users.iter() {
            result.push(itm.clone());
        }

        result
    }

    pub fn add_permanent_user(&mut self, user_id: String) -> Vec<PermanentUserPersistModel> {
        self.users.insert_or_replace(PermanentUserPersistModel {
            user: user_id,
            created: DateTimeAsMicroseconds::now().unix_microseconds,
            status: PermanentUserStatusGrpcModel::InProcess as i32,
        });

        self.get_all()
    }

    pub fn remove_permanent_user(&mut self, user_id: &str) -> Vec<PermanentUserPersistModel> {
        self.users.remove(user_id);
        self.get_all()
    }
}
