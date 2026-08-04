// webrain-core: CDP browser abstraction over Chrome/Lightpanda/Obscura.
//
// One BrowserBackend trait; CdpBackend drives any CDP engine. Engines
// (spider, tile, vision-index) are generic over the trait — no wrapper layer.
//
// ponytail: one trait, one backend, no middleware framework.

pub mod backends;
pub mod browser;
pub mod engines;
pub mod launch;
pub mod vault;
pub mod vision;

pub use backends::cdp::CdpBackend;
// ponytail: TileEngine/TileShot/EmbeddingClient/VectoreStore used internally
// but never imported by external crates — dead re-exports removed.
pub use engines::{CrawlStrategy, SpiderEngine};
pub use vision::{EmbedInput, VectorStore};
