use std::collections::{BTreeMap, HashMap};

use my_ssh::ssh_settings::*;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
pub struct SettingsModel {
    /// The name shown in the UI -> gRPC of the my-telemetry-server of that env
    /// (`ssh:...->http://...` when it is reached through an ssh tunnel).
    pub envs: BTreeMap<String, String>,
    pub ssh_private_keys: Option<HashMap<String, SshPrivateKeySettingsModel>>,
}

impl SettingsModel {
    pub fn get_envs(&self) -> Vec<String> {
        self.envs.keys().cloned().collect()
    }

    pub fn get_env_url(&self, env: &str) -> Option<&str> {
        self.envs.get(env).map(|url| url.as_str())
    }
}

#[async_trait::async_trait]
impl SshSecurityCredentialsResolver for SettingsModel {
    async fn resolve_ssh_private_key(&self, ssh_line: &str) -> Option<SshPrivateKey> {
        let private_keys = self.ssh_private_keys.as_ref()?;

        let ssh_credentials = private_keys
            .get(ssh_line)
            .or_else(|| private_keys.get("*"))?;

        Some(SshPrivateKey {
            content: ssh_credentials.load_cert().await,
            pass_phrase: ssh_credentials.cert_pass_phrase.clone(),
        })
    }

    async fn resolve_ssh_password(&self, _ssh_line: &str) -> Option<String> {
        None
    }
}

#[cfg(test)]
mod test {
    use super::SettingsModel;

    #[test]
    fn reads_the_settings_of_the_readme() {
        let yaml = r#"
envs:
  prod: http://10.0.0.3:8888
  stage: ssh:gateway@10.0.0.1:22->http://10.0.0.5:8888
ssh_private_keys:
  "*":
    cert_path: /root/cert
"#;

        let settings: SettingsModel = serde_yaml::from_str(yaml).unwrap();

        // BTreeMap: the UI lists the envs in this order
        assert_eq!(vec!["prod", "stage"], settings.get_envs());
        assert_eq!(
            Some("ssh:gateway@10.0.0.1:22->http://10.0.0.5:8888"),
            settings.get_env_url("stage")
        );
        assert_eq!(None, settings.get_env_url("dev"));
    }
}
