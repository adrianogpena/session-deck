//! Loads and re-saves `config.json` and `state.json` under `SESSION_DECK_HOME` (or the real
//! `~/.session-deck`), so you can check that nothing is lost on a round trip.

use sdeck_core::store::deck_config::{deck_config_path, read_deck_config, write_deck_config};
use sdeck_core::store::deck_store::DeckStore;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config_path = deck_config_path();
    let config = read_deck_config(&config_path);
    write_deck_config(&config, &config_path)?;
    println!("rewrote {}", config_path.display());

    let store = DeckStore::at_default_path();
    store.resave()?;
    println!("rewrote {}", store.file_path.display());
    Ok(())
}
