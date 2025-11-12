// Phase 4: In-memory texture tracking (no SQLite needed!)

pub mod texture_record;
pub mod discovery;

pub use texture_record::TextureRecord;
pub use discovery::TextureDiscoveryService;
