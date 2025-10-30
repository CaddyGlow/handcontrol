use anyhow::{Context, Result};
use std::fs;
use std::path::Path;
use tracing::{info, warn};
use uuid::Uuid;

use super::paths;

/// Load server ID from persistent storage, or generate a new one if not found
pub fn load_or_create_server_id() -> Result<Uuid> {
    let server_id_path = paths::server_id_path()?;

    if server_id_path.exists() {
        match load_server_id(&server_id_path) {
            Ok(id) => {
                info!("Loaded existing server ID: {}", id);
                return Ok(id);
            }
            Err(e) => {
                warn!(
                    "Failed to load server ID from {}: {}. Generating new ID.",
                    server_id_path.display(),
                    e
                );
            }
        }
    }

    // Generate new server ID
    let server_id = Uuid::new_v4();
    info!("Generated new server ID: {}", server_id);

    // Save it for future use
    if let Err(e) = save_server_id(&server_id_path, &server_id) {
        warn!(
            "Failed to save server ID to {}: {}. Server ID will be regenerated on next restart.",
            server_id_path.display(),
            e
        );
    } else {
        info!("Server ID saved to {}", server_id_path.display());
    }

    Ok(server_id)
}

/// Load server ID from file
fn load_server_id(path: &Path) -> Result<Uuid> {
    let content = fs::read_to_string(path)
        .with_context(|| format!("Failed to read server ID from {}", path.display()))?;

    let id_str = content.trim();
    Uuid::parse_str(id_str)
        .with_context(|| format!("Invalid server ID format in {}: {}", path.display(), id_str))
}

/// Save server ID to file
fn save_server_id(path: &Path, server_id: &Uuid) -> Result<()> {
    // Ensure parent directory exists
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| {
            format!(
                "Failed to create directory for server ID: {}",
                parent.display()
            )
        })?;
    }

    fs::write(path, server_id.to_string())
        .with_context(|| format!("Failed to write server ID to {}", path.display()))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn test_save_and_load_server_id() {
        let temp_file = NamedTempFile::new().unwrap();
        let path = temp_file.path();

        let original_id = Uuid::new_v4();
        save_server_id(path, &original_id).unwrap();

        let loaded_id = load_server_id(path).unwrap();
        assert_eq!(original_id, loaded_id);
    }

    #[test]
    fn test_load_invalid_server_id() {
        let mut temp_file = NamedTempFile::new().unwrap();
        write!(temp_file, "not-a-valid-uuid").unwrap();

        let result = load_server_id(temp_file.path());
        assert!(result.is_err());
    }

    #[test]
    fn test_load_nonexistent_file() {
        let result = load_server_id(Path::new("/nonexistent/path/server_id.txt"));
        assert!(result.is_err());
    }

    #[test]
    fn test_load_or_create_generates_new_id() {
        // This test would require mocking the paths module or using a test directory
        // For now, we'll test the individual functions
        let temp_file = NamedTempFile::new().unwrap();
        let path = temp_file.path();

        // Delete the file to simulate non-existence
        std::fs::remove_file(path).unwrap();

        let id1 = Uuid::new_v4();
        save_server_id(path, &id1).unwrap();

        let id2 = load_server_id(path).unwrap();
        assert_eq!(id1, id2);
    }
}
