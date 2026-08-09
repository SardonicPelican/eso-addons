use clap::Parser;
use eso_addons::{
    addons,
    addons::Manager,
    cache::AddonCache,
    config::{self, AddonEntry, Config},
    htmlparser,
};
use std::path::Path;

use super::{Error, Result};

#[derive(Parser)]
pub struct AddCommand {
    addon_url: Option<String>,
    #[clap(
        short,
        long,
        help = "Indicate, if the addon is only a dependency for another addon"
    )]
    #[clap(short)]
    dependency: bool,
}

impl AddCommand {
    pub fn run(
        &mut self,
        cfg: &mut Config,
        config_filepath: &Path,
        addon_manager: &Manager,
    ) -> Result<()> {
        let mut entry = self.get_entry()?;

        if cfg.addons.iter().find(|el| el.url == entry.url).is_some() {
            println!("Addon {} is already installed", &entry.name);

            // Still check for missing dependencies
            if let Some(addon) = addon_manager
                .get_addon(&entry.name)
                .map_err(|e| Error::Other(Box::new(e)))?
            {
                self.install_dependencies(&addon.depends_on, cfg, config_filepath, addon_manager)?;
            }

            return Ok(());
        }

        let installed = addon_manager.download_addon(&entry.url.clone().unwrap())?;

        if entry.name != installed.name {
            entry.name = installed.name;
        }

        cfg.addons.push(entry.clone());

        config::save_config(config_filepath, &cfg)?;

        println!("🎊 Installed {}!", &entry.name);

        // Install dependencies
        self.install_dependencies(&installed.depends_on, cfg, config_filepath, addon_manager)?;

        Ok(())
    }

    fn install_dependencies(
        &self,
        depends_on: &[String],
        cfg: &mut Config,
        config_filepath: &Path,
        addon_manager: &Manager,
    ) -> Result<()> {
        for dep_name in depends_on {
            // Skip if already in config
            if cfg.addons.iter().any(|el| el.name == *dep_name) {
                continue;
            }

            println!("📦 Installing dependency: {}", dep_name);

            let cache = AddonCache::new();
            let dep_url = match cache.find_addon(dep_name)? {
                Some(url) => url,
                None => {
                    eprintln!(
                        "⚠️  Could not find dependency '{}' in addon cache, skipping",
                        dep_name
                    );
                    continue;
                }
            };

            let download_url = match addons::get_download_url(&dep_url) {
                Some(url) => url,
                None => {
                    eprintln!(
                        "⚠️  Could not resolve download URL for '{}', skipping",
                        dep_name
                    );
                    continue;
                }
            };

            let dep_installed = match addon_manager.download_addon(&download_url) {
                Ok(addon) => addon,
                Err(e) => {
                    eprintln!(
                        "⚠️  Failed to download dependency '{}': {}, skipping",
                        dep_name, e
                    );
                    // The cached URL is no longer valid, drop it so the next
                    // lookup falls back to searching esoui.com again.
                    let _ = cache.invalidate(dep_name);
                    continue;
                }
            };

            let dep_entry = AddonEntry {
                name: dep_installed.name.clone(),
                url: Some(download_url),
                dependency: true,
            };

            cfg.addons.push(dep_entry);
            config::save_config(config_filepath, &cfg)?;

            println!("🎊 Installed dependency {}!", &dep_installed.name);

            // Recurse for transitive dependencies
            self.install_dependencies(
                &dep_installed.depends_on,
                cfg,
                config_filepath,
                addon_manager,
            )?;
        }

        Ok(())
    }

    pub fn get_entry(&mut self) -> Result<AddonEntry> {
        if self.addon_url.is_none() {
            self.ask_for_fields()?;
        }

        let addon_url = self
            .addon_url
            .clone()
            .ok_or(Error::Other("missing addon URL".into()))?;
        let dependency = self.dependency;

        let addon_name = htmlparser::get_document(&addon_url)
            .map(htmlparser::get_addon_name)?
            .ok_or(Error::Other("failed to get addon name".into()))?;

        let download_url = addons::get_download_url(&addon_url);

        Ok(AddonEntry {
            name: addon_name,
            url: download_url,
            dependency: dependency,
        })
    }

    fn ask_for_fields(&mut self) -> Result<()> {
        let questions = vec![
            requestty::Question::input("addon_url")
                .message("URL of the addon on esoui.com")
                .build(),
            requestty::Question::confirm("dependency")
                .message("Is addon only a dependency?")
                .default(false)
                .build(),
        ];

        let answers = requestty::prompt(questions).map_err(|err| Error::Other(Box::new(err)))?;

        if let Some(addon_url) = answers.get("addon_url") {
            self.addon_url = addon_url.as_string().map(|x| x.to_owned());
        };

        if let Some(dependency) = answers.get("dependency") {
            self.dependency = dependency.as_bool().unwrap_or(false);
        };

        Ok(())
    }
}
