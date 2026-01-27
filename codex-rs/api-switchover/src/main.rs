use clap::Parser;
use std::path::PathBuf;

use codex_api_switchover::ApiSwitchoverConfig;
use codex_core::auth::AuthCredentialsStoreMode;
use codex_core::auth::AuthDotJson;
use codex_core::auth::load_auth_dot_json;
use codex_core::auth::save_auth;
use codex_core::config::edit::ConfigEditsBuilder;

#[derive(Debug, Parser)]
#[command(name = "codex-api-switchover")]
#[command(about = "YAML-driven API/provider switchover helper for Codex (widex custom)")]
struct Args {
    /// Path to the switchover YAML config.
    #[arg(long)]
    config: PathBuf,

    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, clap::Subcommand)]
enum Command {
    /// List available Codex profiles in the config.
    List,
    /// Resolve the Codex profile/provider/auth that would be applied for the given model.
    Resolve { model: String },
    /// Apply provider/auth settings to `CODEX_HOME` (writes `config.toml` and/or `auth.json`).
    Apply(ApplyArgs),
}

#[derive(Debug, clap::Args)]
struct ApplyArgs {
    /// Override CODEX_HOME (defaults to $CODEX_HOME or ~/.codex).
    #[arg(long)]
    codex_home: Option<PathBuf>,

    /// Resolve and apply the plan for this model (based on config `models`/`rules`).
    #[arg(long, conflicts_with = "profile")]
    model: Option<String>,

    /// Apply this exact profile id (ignores `models`/`rules`).
    #[arg(long, conflicts_with = "model")]
    profile: Option<String>,

    /// Also write `model = "<slug>"` in config.toml (requires --model).
    #[arg(long, requires = "model")]
    set_model: bool,

    /// If set, write provider selection to config.toml when the resolved plan includes one.
    #[arg(long, default_value_t = true, action = clap::ArgAction::Set)]
    write_config: bool,

    /// If set, write auth.json updates (OPENAI_API_KEY / GEMINI_API_KEY) for keys referenced
    /// by the resolved profile, falling back to `WIDEX_SAVED_API_KEYS` when env vars are unset.
    #[arg(long, default_value_t = true, action = clap::ArgAction::Set)]
    write_auth: bool,

    /// Where Codex stores auth credentials.
    #[arg(long, value_enum, default_value_t = AuthStoreMode::Auto)]
    auth_store: AuthStoreMode,
}

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
enum AuthStoreMode {
    File,
    Keyring,
    Auto,
}

impl AuthStoreMode {
    fn to_core(self) -> AuthCredentialsStoreMode {
        match self {
            Self::File => AuthCredentialsStoreMode::File,
            Self::Keyring => AuthCredentialsStoreMode::Keyring,
            Self::Auto => AuthCredentialsStoreMode::Auto,
        }
    }
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let cfg = ApiSwitchoverConfig::load_from_path(&args.config)?;

    match args.command {
        Command::List => {
            let Some(codex) = cfg.codex.as_ref() else {
                return Ok(());
            };
            for (id, profile) in &codex.profiles {
                if let Some(name) = profile.name.as_deref() {
                    println!("{id}\t{name}");
                } else {
                    println!("{id}");
                }
            }
        }
        Command::Resolve { model } => {
            let plan = cfg.resolve_codex_plan_for_model(&model)?;
            match plan {
                None => {
                    println!("No matching profile for model: {model}");
                }
                Some(plan) => {
                    println!("profile_id: {}", plan.profile_id);
                    if let Some(provider_id) = plan.provider_id.as_deref() {
                        println!("provider_id: {provider_id}");
                    }
                    if plan.auth.wants_openai_api_key {
                        if plan.auth.openai_api_key.is_some() {
                            println!("auth: OPENAI_API_KEY (set)");
                        } else {
                            if let Some(env) = plan.auth.openai_api_key_env.as_deref() {
                                println!(
                                    "auth: OPENAI_API_KEY (missing env: {env}; will use saved if available)"
                                );
                            } else {
                                println!(
                                    "auth: OPENAI_API_KEY (missing env; will use saved if available)"
                                );
                            }
                        }
                    }
                    if plan.auth.wants_gemini_api_key {
                        if plan.auth.gemini_api_key.is_some() {
                            println!("auth: GEMINI_API_KEY (set)");
                        } else {
                            if let Some(env) = plan.auth.gemini_api_key_env.as_deref() {
                                println!(
                                    "auth: GEMINI_API_KEY (missing env: {env}; will use saved if available)"
                                );
                            } else {
                                println!(
                                    "auth: GEMINI_API_KEY (missing env; will use saved if available)"
                                );
                            }
                        }
                    }
                }
            }
        }
        Command::Apply(apply) => {
            let codex_home = apply.codex_home.unwrap_or_else(default_codex_home);
            let auth_store = apply.auth_store.to_core();

            let plan = match (apply.model.as_deref(), apply.profile.as_deref()) {
                (Some(model), None) => cfg
                    .resolve_codex_plan_for_model(model)?
                    .ok_or_else(|| anyhow::anyhow!("No matching profile for model: {model}"))?,
                (None, Some(profile_id)) => cfg
                    .resolve_codex_plan_for_profile_id(profile_id)?
                    .ok_or_else(|| anyhow::anyhow!("Unknown profile id: {profile_id}"))?,
                (None, None) => anyhow::bail!("Must specify either --model or --profile"),
                (Some(_), Some(_)) => unreachable!("clap enforces conflicts"),
            };

            if apply.write_config {
                let mut edits = ConfigEditsBuilder::new(&codex_home);
                if apply.set_model
                    && let Some(model) = apply.model.as_deref()
                {
                    edits = edits.set_model(Some(model), None);
                }
                if let Some(provider_id) = plan.provider_id.as_deref() {
                    edits = edits.set_model_provider(Some(provider_id));
                }
                edits.apply_blocking()?;
            }

            if apply.write_auth
                && (plan.auth.wants_openai_api_key || plan.auth.wants_gemini_api_key)
            {
                let mut auth = load_auth_dot_json(&codex_home, auth_store)
                    .unwrap_or(None)
                    .unwrap_or(AuthDotJson {
                        openai_api_key: None,
                        gemini_api_key: None,
                        widex_saved_api_keys: Default::default(),
                        tokens: None,
                        last_refresh: None,
                    });

                let openai_cache_key = format!("profile:{}:OPENAI_API_KEY", plan.profile_id);
                let gemini_cache_key = format!("profile:{}:GEMINI_API_KEY", plan.profile_id);

                if plan.auth.wants_openai_api_key {
                    if let Some(key) = plan.auth.openai_api_key.clone() {
                        auth.widex_saved_api_keys
                            .insert(openai_cache_key.clone(), key.clone());
                        auth.openai_api_key = Some(key);
                    } else if let Some(saved) = auth.widex_saved_api_keys.get(&openai_cache_key) {
                        auth.openai_api_key = Some(saved.clone());
                    } else {
                        let missing_env = plan
                            .auth
                            .openai_api_key_env
                            .as_deref()
                            .unwrap_or("OPENAI_API_KEY");
                        anyhow::bail!(
                            "Profile `{}` requires OPENAI_API_KEY, but env `{}` was missing and no saved key was found",
                            plan.profile_id,
                            missing_env
                        );
                    }
                }

                if plan.auth.wants_gemini_api_key {
                    if let Some(key) = plan.auth.gemini_api_key.clone() {
                        auth.widex_saved_api_keys
                            .insert(gemini_cache_key.clone(), key.clone());
                        auth.gemini_api_key = Some(key);
                    } else if let Some(saved) = auth.widex_saved_api_keys.get(&gemini_cache_key) {
                        auth.gemini_api_key = Some(saved.clone());
                    } else {
                        let missing_env = plan
                            .auth
                            .gemini_api_key_env
                            .as_deref()
                            .unwrap_or("GEMINI_API_KEY");
                        anyhow::bail!(
                            "Profile `{}` requires GEMINI_API_KEY, but env `{}` was missing and no saved key was found",
                            plan.profile_id,
                            missing_env
                        );
                    }
                }
                save_auth(&codex_home, &auth, auth_store)?;
            }

            println!("Applied profile_id: {}", plan.profile_id);
            if let Some(provider_id) = plan.provider_id.as_deref() {
                println!("provider_id: {provider_id}");
            }
        }
    }

    Ok(())
}

fn default_codex_home() -> PathBuf {
    if let Ok(path) = std::env::var("CODEX_HOME") {
        let trimmed = path.trim();
        if !trimmed.is_empty() {
            return PathBuf::from(trimmed);
        }
    }

    dirs::home_dir()
        .map(|home| home.join(".codex"))
        .unwrap_or_else(|| PathBuf::from(".codex"))
}
