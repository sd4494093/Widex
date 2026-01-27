use anyhow::Context;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::Path;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApiSwitchoverConfig {
    #[serde(default)]
    pub version: Option<u32>,
    #[serde(default)]
    pub codex: Option<CodexSwitchoverConfig>,
}

impl ApiSwitchoverConfig {
    pub fn load_from_path(path: &Path) -> anyhow::Result<Self> {
        let contents = std::fs::read_to_string(path).with_context(|| {
            format!("failed to read api switchover config at {}", path.display())
        })?;
        serde_yaml::from_str(&contents).context("failed to parse api switchover yaml")
    }

    pub fn resolve_codex_plan_for_model(
        &self,
        model: &str,
    ) -> anyhow::Result<Option<ResolvedPlan>> {
        let Some(codex) = self.codex.as_ref() else {
            return Ok(None);
        };
        codex.resolve_plan_for_model(model)
    }

    pub fn resolve_codex_plan_for_profile_id(
        &self,
        profile_id: &str,
    ) -> anyhow::Result<Option<ResolvedPlan>> {
        let Some(codex) = self.codex.as_ref() else {
            return Ok(None);
        };
        codex.resolve_plan_for_profile_id(profile_id)
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CodexSwitchoverConfig {
    #[serde(default)]
    pub profiles: BTreeMap<String, CodexProfile>,

    /// Exact model slug -> profile id.
    #[serde(default)]
    pub models: BTreeMap<String, String>,

    /// Ordered rules; first match wins.
    #[serde(default)]
    pub rules: Vec<ModelRule>,

    /// Fallback profile id when no explicit mapping matches.
    #[serde(default)]
    pub default_profile: Option<String>,
}

impl CodexSwitchoverConfig {
    fn resolve_plan_for_model(&self, model: &str) -> anyhow::Result<Option<ResolvedPlan>> {
        let profile_id = self
            .models
            .get(model)
            .cloned()
            .or_else(|| {
                self.rules
                    .iter()
                    .find(|rule| rule.match_spec.matches(model))
                    .map(|rule| rule.profile.clone())
            })
            .or_else(|| self.default_profile.clone());

        let Some(profile_id) = profile_id else {
            return Ok(None);
        };

        let profile = self.profiles.get(&profile_id).with_context(|| {
            format!("api switchover profile `{profile_id}` was referenced but not defined")
        })?;

        Ok(Some(ResolvedPlan {
            profile_id,
            provider_id: profile.provider_id.clone(),
            auth: profile
                .auth
                .as_ref()
                .map(CodexAuthConfig::resolve)
                .transpose()?
                .unwrap_or_default(),
        }))
    }

    fn resolve_plan_for_profile_id(
        &self,
        profile_id: &str,
    ) -> anyhow::Result<Option<ResolvedPlan>> {
        let profile = match self.profiles.get(profile_id) {
            Some(profile) => profile,
            None => return Ok(None),
        };

        Ok(Some(ResolvedPlan {
            profile_id: profile_id.to_string(),
            provider_id: profile.provider_id.clone(),
            auth: profile
                .auth
                .as_ref()
                .map(CodexAuthConfig::resolve)
                .transpose()?
                .unwrap_or_default(),
        }))
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CodexProfile {
    #[serde(default)]
    pub name: Option<String>,

    /// Key into `model_providers` (e.g. "openai", "gemini", "openai-proxy").
    #[serde(default)]
    pub provider_id: Option<String>,

    /// Optional auth keys to write into `auth.json` (OpenAI and/or Gemini).
    #[serde(default)]
    pub auth: Option<CodexAuthConfig>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelRule {
    #[serde(rename = "match")]
    pub match_spec: ModelMatch,

    #[serde(rename = "use")]
    pub profile: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelMatch {
    #[serde(default)]
    pub equals: Option<String>,
    #[serde(default)]
    pub prefix: Option<String>,
}

impl ModelMatch {
    fn matches(&self, model: &str) -> bool {
        if let Some(exact) = self.equals.as_deref() {
            return model == exact;
        }
        if let Some(prefix) = self.prefix.as_deref() {
            return model.starts_with(prefix);
        }
        false
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CodexAuthConfig {
    #[serde(default)]
    pub openai_api_key: Option<SecretSource>,
    #[serde(default)]
    pub gemini_api_key: Option<SecretSource>,
}

impl CodexAuthConfig {
    fn resolve(&self) -> anyhow::Result<ResolvedAuth> {
        let wants_openai_api_key = self.openai_api_key.is_some();
        let wants_gemini_api_key = self.gemini_api_key.is_some();

        Ok(ResolvedAuth {
            openai_api_key: self
                .openai_api_key
                .as_ref()
                .map(SecretSource::resolve_optional)
                .transpose()?
                .flatten(),
            openai_api_key_env: self
                .openai_api_key
                .as_ref()
                .and_then(SecretSource::env_var_name)
                .map(str::to_string),
            gemini_api_key: self
                .gemini_api_key
                .as_ref()
                .map(SecretSource::resolve_optional)
                .transpose()?
                .flatten(),
            gemini_api_key_env: self
                .gemini_api_key
                .as_ref()
                .and_then(SecretSource::env_var_name)
                .map(str::to_string),
            wants_openai_api_key,
            wants_gemini_api_key,
        })
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ResolvedPlan {
    pub profile_id: String,
    pub provider_id: Option<String>,
    pub auth: ResolvedAuth,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ResolvedAuth {
    pub openai_api_key: Option<String>,
    pub openai_api_key_env: Option<String>,
    pub gemini_api_key: Option<String>,
    pub gemini_api_key_env: Option<String>,
    pub wants_openai_api_key: bool,
    pub wants_gemini_api_key: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum SecretSource {
    /// Literal value (allows config files to embed keys, but avoid committing them).
    Literal(String),
    /// Read from an environment variable.
    Env { env: String },
}

impl SecretSource {
    fn resolve_optional(&self) -> anyhow::Result<Option<String>> {
        match self {
            Self::Literal(value) => Ok(Some(value.clone())),
            Self::Env { env } => {
                let value = std::env::var(env).ok();
                let Some(value) = value else {
                    return Ok(None);
                };
                let trimmed = value.trim();
                if trimmed.is_empty() {
                    return Ok(None);
                }
                Ok(Some(trimmed.to_string()))
            }
        }
    }

    fn env_var_name(&self) -> Option<&str> {
        match self {
            Self::Env { env } => Some(env.as_str()),
            Self::Literal(_) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use tempfile::NamedTempFile;

    #[test]
    fn resolves_exact_model_mapping() {
        let yaml = r#"
version: 1
codex:
  profiles:
    p1:
      provider_id: openai
      auth:
        openai_api_key: "sk-test"
  models:
    gpt-5.1: p1
"#;
        let tmp = NamedTempFile::new().expect("tmpfile");
        std::fs::write(tmp.path(), yaml).expect("write");
        let cfg = ApiSwitchoverConfig::load_from_path(tmp.path()).expect("parse");

        let plan = cfg
            .resolve_codex_plan_for_model("gpt-5.1")
            .expect("resolve")
            .expect("plan");
        assert_eq!(
            plan,
            ResolvedPlan {
                profile_id: "p1".to_string(),
                provider_id: Some("openai".to_string()),
                auth: ResolvedAuth {
                    openai_api_key: Some("sk-test".to_string()),
                    openai_api_key_env: None,
                    gemini_api_key: None,
                    gemini_api_key_env: None,
                    wants_openai_api_key: true,
                    wants_gemini_api_key: false,
                },
            }
        );
    }

    #[test]
    fn resolves_prefix_rule() {
        let yaml = r#"
version: 1
codex:
  profiles:
    gem:
      provider_id: gemini
  rules:
    - match:
        prefix: "gemini-"
      use: gem
"#;
        let tmp = NamedTempFile::new().expect("tmpfile");
        std::fs::write(tmp.path(), yaml).expect("write");
        let cfg = ApiSwitchoverConfig::load_from_path(tmp.path()).expect("parse");

        let plan = cfg
            .resolve_codex_plan_for_model("gemini-2.0-flash")
            .expect("resolve")
            .expect("plan");
        assert_eq!(plan.profile_id, "gem");
        assert_eq!(plan.provider_id.as_deref(), Some("gemini"));
    }

    #[test]
    fn captures_env_var_names_for_configured_keys() {
        let yaml = r#"
version: 1
codex:
  profiles:
    grok:
      provider_id: grok-vectorengine
      auth:
        openai_api_key:
          env: GROK_API_KEY
        gemini_api_key:
          env: GEMINI_API_KEY
  models:
    grok-4.1: grok
"#;
        let tmp = NamedTempFile::new().expect("tmpfile");
        std::fs::write(tmp.path(), yaml).expect("write");
        let cfg = ApiSwitchoverConfig::load_from_path(tmp.path()).expect("parse");

        let plan = cfg
            .resolve_codex_plan_for_model("grok-4.1")
            .expect("resolve")
            .expect("plan");
        assert_eq!(
            plan.auth.openai_api_key_env.as_deref(),
            Some("GROK_API_KEY")
        );
        assert_eq!(
            plan.auth.gemini_api_key_env.as_deref(),
            Some("GEMINI_API_KEY")
        );
        assert!(plan.auth.wants_openai_api_key);
        assert!(plan.auth.wants_gemini_api_key);
    }
}
