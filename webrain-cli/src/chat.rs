//! `webrain chat "<prompt>"` — one command that drives a Copilot session whose
//! model traffic goes to the user's own key and whose tools are *webrain's own
//! MCP server* (attached over stdio, so the agent sees `webrain-webrain_*`).
//!
//! Split on purpose: provider resolution lives in `webrain_core::byok` as pure
//! data (testable with no key and no SDK), and this file is only flag parsing,
//! printing, and — behind `--features chat` — the session itself. A default
//! build still answers `--print-config`, which is the offline gate.
//!
//! ponytail: no clap. `match` + a two-branch argument loop, like the other
//! subcommands in main.rs.

use webrain_core::byok::{ChatFlags, ChatProvider, chat_env_keys, resolve_from};

/// What the user asked for. Every field is optional except the prompt, and the
/// prompt is optional because `--print-config` is a legitimate use on its own.
#[derive(Debug, Default)]
pub struct Args {
    pub prompt: Option<String>,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub base_url: Option<String>,
    pub key_from: Option<String>,
    /// Completion budget. `None` (the default) becomes 4096, not the runtime's
    /// 64k: some providers reserve the requested maximum against your balance
    /// and refuse the request outright when it exceeds what is left.
    pub max_output_tokens: Option<u32>,
    /// Prompt budget handed to the provider. `None` (the default) lets the
    /// runtime use the model's own advertised limit — forcing a value overrides
    /// that knowledge and turns a small-context model into "prompt too long".
    pub max_prompt_tokens: Option<i64>,
    pub timeout_secs: u64,
    pub yes: bool,
    /// `--search`: live web results in front of the turn. Which mechanism
    /// depends on the provider — a `:online` model variant (OpenRouter), the
    /// vendor's own search endpoint (DeepSeek, OpenAI, Anthropic, Gemini), or
    /// webrain's engines. See `ChatProvider::search_mode`.
    pub search: bool,
    pub json: bool,
    pub stream: bool,
    pub print_config: bool,
}

/// Pull the value for a flag, accepting `--x y` and `--x=y`. `i` advances past
/// the value so the caller's loop step stays simple (a closure that captured
/// `i` mutably would fight the loop's own borrow).
fn value_of(
    argv: &[String],
    i: &mut usize,
    flag: &str,
    inline: Option<&str>,
) -> Result<String, String> {
    if let Some(v) = inline {
        return Ok(v.to_string());
    }
    *i += 1;
    argv.get(*i)
        .cloned()
        .ok_or_else(|| format!("{flag} needs a value"))
}

fn parse(argv: &[String]) -> Result<Args, String> {
    let mut a = Args {
        timeout_secs: 300,
        stream: true,
        ..Default::default()
    };
    let mut words: Vec<String> = Vec::new();
    let mut i = 2; // argv[0] = exe, argv[1] = "chat"
    while let Some(arg) = argv.get(i) {
        let (name, inline) = match arg.split_once('=') {
            Some((n, v)) if n.starts_with("--") => (n, Some(v)),
            _ => (arg.as_str(), None),
        };
        match name {
            "--provider" => a.provider = Some(value_of(argv, &mut i, name, inline)?),
            "--model" => a.model = Some(value_of(argv, &mut i, name, inline)?),
            "--base-url" => a.base_url = Some(value_of(argv, &mut i, name, inline)?),
            "--key-from" => a.key_from = Some(value_of(argv, &mut i, name, inline)?),
            "--max-output-tokens" => {
                let v = value_of(argv, &mut i, name, inline)?;
                a.max_output_tokens = Some(
                    v.parse()
                        .map_err(|_| format!("--max-output-tokens {v}: not a number"))?,
                );
            }
            "--max-prompt-tokens" => {
                let v = value_of(argv, &mut i, name, inline)?;
                a.max_prompt_tokens = Some(
                    v.parse()
                        .map_err(|_| format!("--max-prompt-tokens {v}: not a number"))?,
                );
            }
            "--timeout" => {
                let v = value_of(argv, &mut i, name, inline)?;
                a.timeout_secs = v
                    .parse()
                    .map_err(|_| format!("--timeout {v}: not a number of seconds"))?;
            }
            "--yes" => a.yes = true,
            "--search" => a.search = true,
            "--json" => a.json = true,
            "--no-stream" => a.stream = false,
            "--stream" => a.stream = true,
            "--print-config" => a.print_config = true,
            other if other.starts_with('-') => {
                return Err(format!("unknown flag {other} (see `webrain chat --help`)"));
            }
            word => words.push(word.to_string()),
        }
        i += 1;
    }
    if !words.is_empty() {
        a.prompt = Some(words.join(" "));
    }
    Ok(a)
}

/// Env snapshot as the `(name, value)` slice the pure resolver takes. One copy,
/// so `--key-from` and the ambient chain see exactly the same view.
fn env_snapshot() -> Vec<(String, String)> {
    std::env::vars().collect()
}

fn as_refs(env: &[(String, String)]) -> Vec<(&str, &str)> {
    env.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect()
}

/// The redacted render used by `--print-config` and by every error path: the
/// key itself is never printed, only which variable supplied it.
fn config_json(p: &ChatProvider, configured: bool) -> serde_json::Value {
    serde_json::json!({
        "configured": configured,
        "provider_type": p.provider_type,
        "base_url": p.base_url,
        "model": p.model,
        "key_env": p.key_env,
        "api_key": if p.bearer_token().is_some() { "<redacted>" } else { "<none>" },
    })
}

pub fn run(argv: &[String], rt: &tokio::runtime::Runtime) -> anyhow::Result<()> {
    let args = match parse(argv) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("webrain chat: {e}");
            std::process::exit(2);
        }
    };

    let env = env_snapshot();
    let refs = as_refs(&env);
    let flags = ChatFlags {
        provider: args.provider.as_deref(),
        model: args.model.as_deref(),
        base_url: args.base_url.as_deref(),
        key_from: args.key_from.as_deref(),
    };
    let resolved = match resolve_from(&refs, &flags) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("webrain chat: {e}");
            std::process::exit(2);
        }
    };
    // Before `--print-config`, so what it prints is the model that will be
    // sent (`--search` rewrites `openrouter` models to their `:online` slug).
    let resolved = resolved.map(|p| p.with_search(args.search));

    // --print-config is the whole resolution chain, offline and deterministic:
    // it works in a default build precisely so CI can assert it.
    if args.print_config {
        match &resolved {
            Some(p) => println!("{}", serde_json::to_string_pretty(&config_json(p, true))?),
            None => {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "configured": false,
                        "hint": no_key_hint(),
                    }))?
                );
                std::process::exit(1);
            }
        }
        return Ok(());
    }

    let Some(provider) = resolved else {
        eprintln!("webrain chat: {} ", no_key_hint());
        std::process::exit(2);
    };
    let prompt = match args.prompt.as_deref() {
        Some(p) if !p.trim().is_empty() => p,
        _ => {
            eprintln!("webrain chat: give a prompt, e.g.  webrain chat \"list the files in docs\"");
            std::process::exit(2);
        }
    };

    turn(rt, &args, &provider, prompt)
}

/// One sentence naming the variables that would have worked, from the live
/// environment (a stale hard-coded list is worse than none).
fn no_key_hint() -> String {
    let have = chat_env_keys();
    if have.is_empty() {
        "no chat provider configured: set OPENROUTER_API_KEY, OPENAI_API_KEY, \
         ANTHROPIC_API_KEY, GEMINI_API_KEY, FIREWORKS_API_KEY, DASHSCOPE_API_KEY or \
         GROQ_API_KEY, or WEBRAIN_CHAT_BASE_URL for a local server"
            .to_string()
    } else {
        // Keys exist but nothing resolved: the only way here is a provider name
        // that does not match, and the resolver already said which.
        format!("could not resolve a chat provider from {}", have[0].0)
    }
}

#[cfg(feature = "chat")]
fn turn(
    rt: &tokio::runtime::Runtime,
    args: &Args,
    provider: &ChatProvider,
    prompt: &str,
) -> anyhow::Result<()> {
    match rt.block_on(super::chat_session::run(args, provider, prompt)) {
        Ok(()) => Ok(()),
        Err(e) => {
            // 1 = the turn failed; 2 stays reserved for "cannot start at all"
            // (no key, no runtime, bad flags) so a caller can tell them apart.
            eprintln!("webrain chat: {}", advise(&e.to_string()));
            std::process::exit(1);
        }
    }
}

/// The runtime reports every rejected request as "the prompt was too long", but
/// the causes need opposite fixes and neither is the context window people
/// assume. Measured, on this repo's own runs: Groq's free tier caps input at
/// 7000 tokens/minute and a tool-enabled agent prompt is ~8.4k (mostly the tool
/// schemas), and OpenRouter rejected a turn because the runtime asked to
/// reserve 64000 output tokens against a nearly empty balance.
#[cfg(feature = "chat")]
fn advise(msg: &str) -> String {
    let m = msg.to_ascii_lowercase();
    let quota = m.contains("too long") || m.contains("too large") || m.contains("reduce context");
    if !quota {
        return msg.to_string();
    }
    format!(
        "{msg}\n  \
         That is the provider refusing the request, not a webrain failure. Usual causes:\n  \
         - tokens-per-minute cap. Free tiers are small (Groq: 7000 input tokens/min) and the \
         tool schemas alone are ~8k. Use a paid tier or another provider.\n  \
         - credit balance below the reserved output budget. Lower --max-output-tokens \
         (default 4096) or top up.\n  \
         Re-run with WEBRAIN_CHAT_DEBUG=1 to see the provider's own status code and message."
    )
}

#[cfg(not(feature = "chat"))]
fn turn(
    _rt: &tokio::runtime::Runtime,
    _args: &Args,
    _provider: &ChatProvider,
    _prompt: &str,
) -> anyhow::Result<()> {
    eprintln!(
        "webrain chat: this build has no chat runtime.\n  \
         rebuild with:  cargo build --release --features chat\n  \
         provider resolved fine ({}/{}) — only the Copilot runtime is missing.",
        _provider.provider_type, _provider.model
    );
    std::process::exit(2);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(s: &[&str]) -> Vec<String> {
        let mut v = vec!["webrain".to_string(), "chat".to_string()];
        v.extend(s.iter().map(|x| x.to_string()));
        v
    }

    #[test]
    fn positional_words_join_into_one_prompt() {
        let a = parse(&argv(&[
            "scrape",
            "news.ycombinator.com",
            "and",
            "summarise",
        ]))
        .unwrap();
        assert_eq!(
            a.prompt.as_deref(),
            Some("scrape news.ycombinator.com and summarise")
        );
    }

    #[test]
    fn value_flags_work_in_both_spellings() {
        let a = parse(&argv(&[
            "--provider=groq",
            "--model",
            "m1",
            "hi",
            "--max-output-tokens",
            "512",
        ]))
        .unwrap();
        assert_eq!(a.provider.as_deref(), Some("groq"));
        assert_eq!(a.model.as_deref(), Some("m1"));
        assert_eq!(a.max_output_tokens, Some(512));
        assert_eq!(a.prompt.as_deref(), Some("hi"));
        assert!(a.stream, "streaming is the default");
    }

    #[test]
    fn booleans_and_unknown_flags() {
        let a = parse(&argv(&["--yes", "--json", "--no-stream", "hi"])).unwrap();
        assert!(a.yes && a.json && !a.stream);
        let e = parse(&argv(&["--nope"])).unwrap_err();
        assert!(e.contains("unknown flag --nope"), "{e}");
    }

    #[test]
    fn missing_value_is_an_error_not_a_silent_default() {
        let e = parse(&argv(&["--provider"])).unwrap_err();
        assert!(e.contains("--provider needs a value"), "{e}");
    }

    #[test]
    fn print_config_json_never_contains_the_key() {
        // Built through the real resolver, so the assertion covers the path the
        // binary actually takes (bearer is private to byok on purpose).
        let env = [("OPENAI_API_KEY", "sk-do-not-print")];
        let p = resolve_from(&env, &ChatFlags::default()).unwrap().unwrap();
        let s = serde_json::to_string(&config_json(&p, true)).unwrap();
        assert!(!s.contains("sk-do-not-print"), "{s}");
        assert!(s.contains("<redacted>"), "{s}");
        assert!(s.contains("\"key_env\":\"OPENAI_API_KEY\""), "{s}");
    }
}
