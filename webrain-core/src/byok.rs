//! Bring-your-own-key chat provider resolution for `webrain chat`.
//!
//! Same shape as `video::VISION_PROVIDERS`: a table of
//! `(env aliases, base_url, model)` rows resolved first-match-wins, so the two
//! chains cannot drift in style and a provider renaming a model is a one-line
//! change. Everything here is pure — `resolve_from` takes a `(name, value)`
//! slice instead of reading the environment, so every row is testable without a
//! key, a network call, or the Copilot SDK.
//!
//! `base_url` is a BASE url (the CLI appends `/chat/completions` or `/messages`
//! per `provider_type`), which is what `ProviderConfig::base_url` expects.

/// Rows are `(env-name aliases, OpenAI-compatible base url, default model)`.
/// Order is the ambient fallback order when the user sets several keys.
///
/// `provider_type` is derived from the row, not stored: only Anthropic speaks a
/// non-OpenAI wire format, and every other row is an OpenAI-compatible gateway.
pub const CHAT_PROVIDERS: [(&[&str], &str, &str); 8] = [
    (
        &["OPENROUTER_API_KEY"],
        "https://openrouter.ai/api/v1",
        "anthropic/claude-sonnet-4.5",
    ),
    (&["OPENAI_API_KEY"], "https://api.openai.com/v1", "gpt-5"),
    (
        &["ANTHROPIC_API_KEY", "CLAUDE_API_KEY"],
        "https://api.anthropic.com/v1",
        "claude-sonnet-4-5",
    ),
    (
        &["GEMINI_API_KEY", "GOOGLE_API_KEY"],
        "https://generativelanguage.googleapis.com/v1beta/openai",
        "gemini-2.5-pro",
    ),
    (
        &["FIREWORKS_API_KEY"],
        "https://api.fireworks.ai/inference/v1",
        "accounts/fireworks/models/qwen3-235b-a22b",
    ),
    (
        &["DASHSCOPE_API_KEY", "QWEN_API_KEY"],
        "https://dashscope-intl.aliyuncs.com/compatible-mode/v1",
        "qwen3-max",
    ),
    (
        &["GROQ_API_KEY"],
        "https://api.groq.com/openai/v1",
        // Groq never hosted the Qwen 3.6 id the vision table once used; its
        // catalog lists 3.8 (live 2026-11-03).
        "qwen/qwen3.8-27b",
    ),
    (
        &["DEEPSEEK_API_KEY"],
        // No `/v1`: DeepSeek's OpenAI base url is the host root and its own
        // docs show `https://api.deepseek.com/chat/completions`.
        "https://api.deepseek.com",
        // `deepseek-flash` is the current name (served by V4.1-Flash); the
        // legacy `deepseek-v4-flash` is still accepted. `deepseek-v4-pro` is
        // the other catalog entry, reachable with --model.
        "deepseek-flash",
    ),
];

/// The `Chat` group of `webrain help env`, defined here so the variables the
/// resolver actually reads and the variables the terminal advertises are one
/// list. `webrain-cli` splices this into its `ENV` table.
pub const CHAT_ENV_GROUP: (&str, &[(&str, &str)]) = (
    "Chat (Copilot BYOK)",
    &[
        (
            "WEBRAIN_CHAT_PROVIDER",
            "row to use: openrouter, openai, anthropic/claude, gemini/google, fireworks, dashscope/qwen, groq, deepseek",
        ),
        ("WEBRAIN_CHAT_MODEL", "override the row's default model id"),
        (
            "WEBRAIN_CHAT_BASE_URL",
            "override the row's base url (a keyless local server is fine)",
        ),
        (
            "WEBRAIN_CHAT_API_KEY",
            "the key itself, for one variable instead of a vendor-named one",
        ),
        (
            "OPENROUTER_API_KEY / OPENAI_API_KEY",
            "chat rows 1-2 (ambient key chain order)",
        ),
        ("ANTHROPIC_API_KEY / CLAUDE_API_KEY", "chat row 3 (Claude)"),
        ("GEMINI_API_KEY / GOOGLE_API_KEY", "chat row 4 (Gemini)"),
        ("FIREWORKS_API_KEY", "chat row 5"),
        ("DASHSCOPE_API_KEY / QWEN_API_KEY", "chat row 6 (Qwen)"),
        ("GROQ_API_KEY", "chat row 7"),
        (
            "DEEPSEEK_API_KEY",
            "chat row 8 (deepseek-flash; --model deepseek-v4-pro)",
        ),
        (
            "COPILOT_CLI_PATH",
            "use a Copilot runtime you already have instead of the bundled one",
        ),
        (
            "COPILOT_CLI_EXTRACT_DIR",
            "where the bundled runtime is unpacked",
        ),
        (
            "BUNDLED_CLI_CACHE_DIR",
            "build-time cache of the runtime archives (CI)",
        ),
    ],
);

/// A resolved provider. `bearer` is private on purpose: the SDK wants the
/// secret, but `{:?}` on a config struct must never print it.
#[derive(Clone, PartialEq, Eq)]
pub struct ChatProvider {
    /// Wire format the SDK expects: `openai` | `anthropic`.
    pub provider_type: &'static str,
    /// Base url, no `/chat/completions` suffix.
    pub base_url: String,
    pub model: String,
    /// Variable the key came from (None for a keyless local endpoint), so two
    /// keys stay distinguishable in `--print-config` and `doctor`.
    pub key_env: Option<String>,
    bearer: Option<String>,
}

/// How `--search` is served for a resolved provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchMode {
    /// No `--search`, or the provider has no search at all.
    Off,
    /// The provider searches the web itself: OpenRouter's `:online` model
    /// variant (native search where the vendor has it, Exa otherwise).
    ModelVariant,
    /// Search is a separate REST endpoint that must run before the turn:
    /// DeepSeek's `POST /v1/tools/search`.
    DeepSeekApi,
}

impl ChatProvider {
    /// The secret. Callers hand it to `ProviderConfig::bearer_token` and
    /// nothing else.
    pub fn bearer_token(&self) -> Option<&str> {
        self.bearer.as_deref()
    }

    /// Which search mechanism this provider has. Only the two shapes that
    /// exist in the wild are modelled: a model variant (OpenRouter) and a
    /// separate search endpoint (DeepSeek). Everything else has no search
    /// product, and `with_search` says so instead of pretending.
    pub fn search_mode(&self) -> SearchMode {
        if self.base_url.contains("openrouter.ai") {
            SearchMode::ModelVariant
        } else if self.base_url.contains("api.deepseek.com") {
            SearchMode::DeepSeekApi
        } else {
            SearchMode::Off
        }
    }

    /// `--search`: turn on provider-native web search.
    ///
    /// OpenRouter is rewritten to its `:online` slug (a model id is the only
    /// per-turn field this SDK lets us change). DeepSeek is left alone here
    /// because its search is an endpoint the caller must hit — see
    /// `search_mode`. Any other row has no search to switch on, and saying so
    /// beats a flag that silently does nothing.
    pub fn with_search(self, search: bool) -> Result<Self, String> {
        if !search {
            return Ok(self);
        }
        match self.search_mode() {
            SearchMode::ModelVariant if self.model.ends_with(":online") => Ok(self),
            SearchMode::ModelVariant => Ok(Self {
                model: format!("{}:online", self.model),
                ..self
            }),
            SearchMode::DeepSeekApi => Ok(self),
            SearchMode::Off => Err(format!(
                "--search needs a provider with a search product (openrouter's :online, or deepseek's /v1/tools/search). \
                 {} has none. Drop --search, or let the agent call webrain_search / webrain_serp, which are always attached.",
                self.key_env.as_deref().unwrap_or("This provider")
            )),
        }
    }
}

impl std::fmt::Debug for ChatProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChatProvider")
            .field("provider_type", &self.provider_type)
            .field("base_url", &self.base_url)
            .field("model", &self.model)
            .field("key_env", &self.key_env)
            .field(
                "bearer",
                &if self.bearer.is_some() {
                    "<redacted>"
                } else {
                    "<none>"
                },
            )
            .finish()
    }
}

/// CLI flags that outrank the environment.
#[derive(Debug, Default, Clone, Copy)]
pub struct ChatFlags<'a> {
    pub provider: Option<&'a str>,
    pub model: Option<&'a str>,
    pub base_url: Option<&'a str>,
    /// Variable NAME to read the key from (`--key-from`), resolved against the
    /// same slice `resolve_from` receives.
    pub key_from: Option<&'a str>,
}

/// Row lookup stem: `OPENAI_API_KEY` -> `openai`, `CLAUDE_API_KEY` -> `claude`.
/// `--provider` accepts any alias stem of a row, so `claude` and `anthropic`
/// both work.
fn stems(row: &(&'static [&'static str], &'static str, &'static str)) -> Vec<String> {
    row.0
        .iter()
        .map(|n| n.trim_end_matches("_API_KEY").to_ascii_lowercase())
        .collect()
}

/// Every accepted `--provider` value, for the unknown-provider error.
pub fn provider_names() -> Vec<String> {
    let mut out: Vec<String> = CHAT_PROVIDERS.iter().flat_map(stems).collect();
    out.sort();
    out.dedup();
    out
}

fn find(name: &str) -> Option<&'static (&'static [&'static str], &'static str, &'static str)> {
    let want = name.trim().to_ascii_lowercase();
    CHAT_PROVIDERS.iter().find(|row| stems(row).contains(&want))
}

/// Only Anthropic is not OpenAI-compatible; the alias decides, so `--provider
/// claude` and the `CLAUDE_API_KEY` row agree.
fn provider_type(row: &(&'static [&'static str], &'static str, &'static str)) -> &'static str {
    let s = stems(row);
    if s.iter().any(|n| n == "anthropic" || n == "claude") {
        "anthropic"
    } else {
        "openai"
    }
}

fn env_get<'a>(env: &[(&str, &'a str)], name: &str) -> Option<&'a str> {
    env.iter()
        .find(|(n, _)| *n == name)
        .map(|(_, v)| v.trim())
        .filter(|v| !v.is_empty())
}

/// First present alias of a row: `(env name, value)`.
fn row_key<'a>(
    env: &[(&str, &'a str)],
    row: &'static (&'static [&'static str], &'static str, &'static str),
) -> Option<(&'static str, &'a str)> {
    row.0
        .iter()
        .find_map(|name| env_get(env, name).map(|v| (*name, v)))
}

/// Resolve one provider from flags, then `WEBRAIN_CHAT_*`, then the ambient key
/// chain. `Ok(None)` means nothing is configured (the caller says how to fix
/// it); `Err` means the user's own input is wrong (unknown provider, a
/// `--key-from` naming an unset variable) and must not be papered over.
pub fn resolve_from(
    env: &[(&str, &str)],
    flags: &ChatFlags,
) -> Result<Option<ChatProvider>, String> {
    let chat_key = env_get(env, "WEBRAIN_CHAT_API_KEY");
    let chat_url = env_get(env, "WEBRAIN_CHAT_BASE_URL");
    let chat_model = env_get(env, "WEBRAIN_CHAT_MODEL");

    // 1. The row: --provider, then WEBRAIN_CHAT_PROVIDER, then ambient order.
    let named = flags
        .provider
        .or_else(|| env_get(env, "WEBRAIN_CHAT_PROVIDER"));
    let row = match named {
        Some(name) => Some(find(name).ok_or_else(|| {
            format!(
                "unknown chat provider '{name}' (known: {})",
                provider_names().join(", ")
            )
        })?),
        None => None,
    };

    // 2. The key: --key-from, then WEBRAIN_CHAT_API_KEY, then the row's aliases.
    let (key_env, key): (Option<String>, Option<&str>) = match flags.key_from {
        Some(var) => (
            Some(var.to_string()),
            Some(
                env_get(env, var)
                    .ok_or_else(|| format!("--key-from {var}: that variable is empty or unset"))?,
            ),
        ),
        None => match chat_key {
            Some(k) => (Some("WEBRAIN_CHAT_API_KEY".to_string()), Some(k)),
            None => {
                let ambient = match row {
                    Some(row) => row_key(env, row),
                    None => CHAT_PROVIDERS.iter().find_map(|r| row_key(env, r)),
                };
                match ambient {
                    Some((name, k)) => (Some(name.to_string()), Some(k)),
                    None => (None, None),
                }
            }
        },
    };

    let has_url = flags.base_url.is_some() || chat_url.is_some();
    if key.is_none() && !has_url {
        // A named provider with no key is a typo, not a request to run
        // keyless; anything else is simply "not configured yet".
        if let Some(name) = named {
            return Err(format!(
                "provider '{name}' needs a key: set {} or WEBRAIN_CHAT_API_KEY",
                row.map(|r| r.0.join(" / ")).unwrap_or_default(),
            ));
        }
        return Ok(None);
    }

    // A bare key with no named provider belongs to the row that owns it.
    let fallback = row
        .or_else(|| {
            key_env.as_deref().and_then(|n| {
                CHAT_PROVIDERS
                    .iter()
                    .find(|r| r.0.iter().any(|a| a.eq_ignore_ascii_case(n)))
            })
        })
        .unwrap_or(&CHAT_PROVIDERS[1]); // openai row

    Ok(Some(ChatProvider {
        provider_type: provider_type(fallback),
        base_url: flags
            .base_url
            .or(chat_url)
            .unwrap_or(fallback.1)
            .to_string(),
        model: flags.model.or(chat_model).unwrap_or(fallback.2).to_string(),
        key_env,
        bearer: key.map(str::to_string),
    }))
}

/// Configured keys in table order — for `doctor` and the "nothing set" hint.
pub fn chat_env_keys() -> Vec<(&'static str, String)> {
    CHAT_PROVIDERS
        .iter()
        .filter_map(|row| {
            row.0.iter().find_map(|n| {
                std::env::var(n)
                    .ok()
                    .filter(|v| !v.trim().is_empty())
                    .map(|v| (*n, v))
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(pairs: &[(&'static str, &'static str)]) -> Vec<(&'static str, &'static str)> {
        pairs.to_vec()
    }

    #[test]
    fn every_row_resolves_to_its_own_base_url_and_model() {
        for row in CHAT_PROVIDERS.iter() {
            let (name, _, model) = row;
            let var = name[0];
            let e = env(&[(var, "k-test")]);
            let p = resolve_from(&e, &ChatFlags::default())
                .expect("ok")
                .unwrap_or_else(|| panic!("{var} did not resolve"));
            assert_eq!(p.base_url, row.1, "{var} base url");
            assert_eq!(p.model, *model, "{var} model");
            assert_eq!(p.key_env.as_deref(), Some(var));
            assert_eq!(p.bearer_token(), Some("k-test"));
            assert_eq!(
                p.provider_type,
                if var.starts_with("ANTHROPIC") || var.starts_with("CLAUDE") {
                    "anthropic"
                } else {
                    "openai"
                },
                "{var} wire format"
            );
        }
    }

    #[test]
    fn alias_pairs_reach_the_same_row() {
        let pairs = [
            ("ANTHROPIC_API_KEY", "CLAUDE_API_KEY"),
            ("GEMINI_API_KEY", "GOOGLE_API_KEY"),
            ("DASHSCOPE_API_KEY", "QWEN_API_KEY"),
        ];
        for (a, b) in pairs {
            let pa = resolve_from(&env(&[(a, "k1")]), &ChatFlags::default())
                .unwrap()
                .unwrap();
            let pb = resolve_from(&env(&[(b, "k1")]), &ChatFlags::default())
                .unwrap()
                .unwrap();
            assert_eq!(pa.base_url, pb.base_url, "{a} vs {b}");
            assert_eq!(pb.key_env.as_deref(), Some(b));
        }
    }

    #[test]
    fn ambient_order_is_the_table_order() {
        let e = env(&[("GROQ_API_KEY", "g"), ("OPENROUTER_API_KEY", "o")]);
        let p = resolve_from(&e, &ChatFlags::default()).unwrap().unwrap();
        assert_eq!(p.key_env.as_deref(), Some("OPENROUTER_API_KEY"));
    }

    #[test]
    fn nothing_configured_is_none_not_error() {
        assert!(resolve_from(&[], &ChatFlags::default()).unwrap().is_none());
    }

    #[test]
    fn unknown_provider_is_an_error_naming_the_known_ones() {
        let flags = ChatFlags {
            provider: Some("nope"),
            ..Default::default()
        };
        let err = resolve_from(&env(&[("OPENAI_API_KEY", "k")]), &flags).unwrap_err();
        assert!(err.contains("unknown chat provider 'nope'"));
        assert!(err.contains("openrouter"));
        assert!(err.contains("groq"));
    }

    #[test]
    fn provider_flag_picks_the_row_and_flags_override_every_field() {
        let flags = ChatFlags {
            provider: Some("groq"),
            model: Some("custom-model"),
            base_url: Some("http://127.0.0.1:9/v1"),
            ..Default::default()
        };
        let p = resolve_from(&env(&[("OPENAI_API_KEY", "k")]), &flags)
            .unwrap()
            .unwrap();
        assert_eq!(p.base_url, "http://127.0.0.1:9/v1");
        assert_eq!(p.model, "custom-model");
        // A named row takes ONLY its own aliases: shipping an OpenAI key to
        // Groq would authenticate nothing, so an explicit base url is what
        // makes this keyless call legal.
        assert_eq!(p.key_env, None);
        assert_eq!(p.bearer_token(), None);
    }

    #[test]
    fn named_provider_never_borrows_another_providers_key() {
        let flags = ChatFlags {
            provider: Some("groq"),
            ..Default::default()
        };
        let err = resolve_from(&env(&[("OPENAI_API_KEY", "sk-openai")]), &flags).unwrap_err();
        assert!(err.contains("needs a key"), "{err}");
        assert!(err.contains("GROQ_API_KEY"), "{err}");
    }

    #[test]
    fn key_from_names_the_variable_and_errors_when_unset() {
        let flags = ChatFlags {
            provider: Some("openai"),
            key_from: Some("MY_KEY"),
            ..Default::default()
        };
        assert!(resolve_from(&[], &flags).is_err());
        let p = resolve_from(&env(&[("MY_KEY", "sk-x")]), &flags)
            .unwrap()
            .unwrap();
        assert_eq!(p.bearer_token(), Some("sk-x"));
        assert_eq!(p.key_env.as_deref(), Some("MY_KEY"));
    }

    #[test]
    fn a_bare_base_url_resolves_keyless_for_local_servers() {
        let e = vec![("WEBRAIN_CHAT_BASE_URL", "http://127.0.0.1:11434/v1")];
        let p = resolve_from(&e, &ChatFlags::default()).unwrap().unwrap();
        assert_eq!(p.base_url, "http://127.0.0.1:11434/v1");
        assert_eq!(p.bearer_token(), None);
        assert_eq!(p.provider_type, "openai");
    }

    #[test]
    fn explicit_provider_without_any_key_is_an_error() {
        let flags = ChatFlags {
            provider: Some("anthropic"),
            ..Default::default()
        };
        let err = resolve_from(&[], &flags).unwrap_err();
        assert!(err.contains("needs a key"), "{err}");
        assert!(err.contains("ANTHROPIC_API_KEY"), "{err}");
    }

    #[test]
    fn debug_never_prints_the_secret() {
        let p = resolve_from(
            &env(&[("OPENAI_API_KEY", "sk-super-secret")]),
            &ChatFlags::default(),
        )
        .unwrap()
        .unwrap();
        let rendered = format!("{p:?}");
        assert!(!rendered.contains("sk-super-secret"), "{rendered}");
        assert!(rendered.contains("<redacted>"), "{rendered}");
    }

    #[test]
    fn chat_env_knobs_are_honoured() {
        let e = vec![
            ("WEBRAIN_CHAT_API_KEY", "k"),
            ("WEBRAIN_CHAT_PROVIDER", "dashscope"),
            ("WEBRAIN_CHAT_MODEL", "qwen-custom"),
        ];
        let p = resolve_from(&e, &ChatFlags::default()).unwrap().unwrap();
        assert!(p.base_url.contains("dashscope"), "{}", p.base_url);
        assert_eq!(p.model, "qwen-custom");
    }

    #[test]
    fn deepseek_row_takes_both_catalog_models() {
        let flags = ChatFlags {
            provider: Some("deepseek"),
            ..Default::default()
        };
        let p = resolve_from(&env(&[("DEEPSEEK_API_KEY", "k")]), &flags)
            .unwrap()
            .unwrap();
        assert_eq!(p.base_url, "https://api.deepseek.com");
        assert_eq!(p.model, "deepseek-flash");
        assert_eq!(p.provider_type, "openai");

        let pro = ChatFlags {
            model: Some("deepseek-v4-pro"),
            ..flags
        };
        let p = resolve_from(&env(&[("DEEPSEEK_API_KEY", "k")]), &pro)
            .unwrap()
            .unwrap();
        assert_eq!(p.model, "deepseek-v4-pro");
        assert_eq!(p.base_url, "https://api.deepseek.com");
    }

    #[test]
    fn search_appends_online_once_on_openrouter() {
        let p = resolve_from(&env(&[("OPENROUTER_API_KEY", "k")]), &ChatFlags::default())
            .unwrap()
            .unwrap();
        let plain = p.model.clone();
        let searched = p.clone().with_search(true).unwrap();
        assert_eq!(searched.model, "anthropic/claude-sonnet-4.5:online");
        // Idempotent: `--search` twice (or a model the user already marked)
        // must not become `:online:online`.
        let again = searched.clone().with_search(true).unwrap();
        assert_eq!(again.model, searched.model);
        assert_eq!(p.with_search(false).unwrap().model, plain);
    }

    #[test]
    fn search_errors_where_the_provider_has_no_search_product() {
        // Groq has no search endpoint and no `:online` variant, so `--search`
        // must refuse rather than quietly do nothing.
        let p = resolve_from(&env(&[("GROQ_API_KEY", "k")]), &ChatFlags::default())
            .unwrap()
            .unwrap();
        let err = p.with_search(true).unwrap_err();
        assert!(err.contains("openrouter"), "{err}");
        assert!(err.contains("GROQ_API_KEY"), "{err}");
    }

    #[test]
    fn search_mode_matches_the_providers_shape() {
        let ds = resolve_from(&env(&[("DEEPSEEK_API_KEY", "k")]), &ChatFlags::default())
            .unwrap()
            .unwrap();
        assert_eq!(ds.search_mode(), SearchMode::DeepSeekApi);
        // DeepSeek's model id is untouched: the search runs as its own call.
        assert_eq!(
            ds.clone().with_search(true).unwrap().model,
            "deepseek-flash"
        );

        let or = resolve_from(&env(&[("OPENROUTER_API_KEY", "k")]), &ChatFlags::default())
            .unwrap()
            .unwrap();
        assert_eq!(or.search_mode(), SearchMode::ModelVariant);
    }
}
