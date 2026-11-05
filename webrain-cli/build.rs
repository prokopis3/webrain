//! Hash the chat UI.
//!
//! The page is served from disk, not from the binary (`src/chat_ui.rs`), so the
//! binary has to be able to prove the files it finds are the ones it shipped.
//! The digests are computed here, at build time, from the same `assets/` the
//! `bun run build` wrote — which is why they cannot drift: rebuilding the UI
//! changes this constant and the old archive stops verifying.

use sha2::{Digest, Sha256};
use std::path::Path;

/// Every file the server answers with, in the order `chat_ui::Ui` holds them.
const FILES: [&str; 4] = ["chat.html", "chat.css", "chat.js", "chunk-markdown.js"];

fn main() {
    println!("cargo:rerun-if-changed=assets");
    let mut out = String::from("pub const MANIFEST: [(&str, &str, u64); 4] = [\n");
    for name in FILES {
        let path = Path::new("assets").join(name);
        println!("cargo:rerun-if-changed={}", path.display());
        // A missing asset is a build error, not a warning: a binary that cannot
        // serve a page should fail here rather than at the first `--serve`.
        let bytes = match std::fs::read(&path) {
            Ok(b) => b,
            Err(e) => panic!(
                "{}: {e} — run `cd ui && bun run build` to write the chat UI assets",
                path.display()
            ),
        };
        let sha = Sha256::digest(&bytes);
        let hex: String = sha.iter().map(|b| format!("{b:02x}")).collect();
        out.push_str(&format!("    (\"{name}\", \"{hex}\", {}),\n", bytes.len()));
    }
    out.push_str("];\n");
    let dest = Path::new(&std::env::var("OUT_DIR").expect("OUT_DIR")).join("chat_ui_manifest.rs");
    std::fs::write(&dest, out).expect("write chat_ui_manifest.rs");
}
