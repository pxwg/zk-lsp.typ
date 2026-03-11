use std::path::PathBuf;

use anyhow::{Context, Result};
use chrono::Local;
use tokio::fs;

use crate::config::WikiConfig;
use crate::link_gen;

/// Create a new note with the current timestamp as ID.
/// Returns the path to the new file.
pub async fn create_note(config: &WikiConfig) -> Result<PathBuf> {
    let id = Local::now().format("%y%m%d%H%M").to_string();
    fs::create_dir_all(&config.note_dir).await?;

    let path = config.note_dir.join(format!("{id}.typ"));
    if !path.exists() {
        let content = format!(
            "#import \"../include.typ\": *\n\
             #let metadata = toml(bytes(\n\
             \x20 ```toml\n\
             \x20 schema-version = 1\n\
             \x20 aliases = []\n\
             \x20 abstract = \"\"\n\
             \x20 keywords = []\n\
             \x20 generated = true\n\
             \x20 checklist-status = \"none\"\n\
             \x20 relation = \"active\"\n\
             \x20 relation-target = []\n\
             \x20 ```.text,\n\
             ))\n\
             #show: zettel.with(metadata: metadata)\n\
             \n\
             =  <{id}>\n"
        );
        fs::write(&path, &content)
            .await
            .with_context(|| format!("writing note {}", path.display()))?;
    }

    link_gen::add_entry(&id, config).await?;
    Ok(path)
}

/// Delete a note and remove its entry from link.typ.
pub async fn delete_note(id: &str, config: &WikiConfig) -> Result<()> {
    let path = config.note_dir.join(format!("{id}.typ"));
    if path.exists() {
        fs::remove_file(&path)
            .await
            .with_context(|| format!("deleting note {}", path.display()))?;
    }
    link_gen::remove_entry(id, config).await?;
    Ok(())
}
