use clap::Parser;
use eso_addons::{
    addons,
    cache::AddonCache,
    config::{self, AddonEntry, Config},
};
use std::path::Path;

use super::Result;

#[derive(Parser)]
pub struct SyncCommand;

impl SyncCommand {
    pub fn run(
        &self,
        cfg: &mut Config,
        config_filepath: &Path,
        addon_manager: &addons::Manager,
    ) -> Result<()> {
        let found = addon_manager.get_addons()?;
        for error in found.errors {
            eprintln!("Warning: {}", error);
        }

        let cache = AddonCache::new();
        let mut changed = false;

        for addon in found.addons {
            let is_dependency = addon.name.starts_with("Lib");

            if let Some(entry) = cfg.addons.iter_mut().find(|entry| entry.name == addon.name) {
                if is_dependency && !entry.dependency {
                    entry.dependency = true;
                    changed = true;
                }
                continue;
            }

            let page_url = match cache.find_addon(&addon.name)? {
                Some(url) => url,
                None => {
                    eprintln!("Warning: could not find '{}' on esoui.com", addon.name);
                    continue;
                }
            };
            let download_url = match addons::get_download_url(&page_url) {
                Some(url) => url,
                None => {
                    eprintln!(
                        "Warning: could not resolve download URL for '{}'",
                        addon.name
                    );
                    continue;
                }
            };

            cfg.addons.push(AddonEntry {
                name: addon.name.clone(),
                url: Some(download_url),
                dependency: is_dependency,
            });
            changed = true;
            println!(
                "Added {}{}",
                addon.name,
                if is_dependency { " (dependency)" } else { "" }
            );
        }

        if changed {
            config::save_config(config_filepath, cfg)?;
        }
        Ok(())
    }
}
