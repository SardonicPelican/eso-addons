use crate::errors::{Error, Result};
use scraper::{Html, Selector};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

const CACHE_DIR_NAME: &str = "eso-addons";
const ADDONS_JSON: &str = "addons.json";
const SEARCH_URL: &str = "https://www.esoui.com/downloads/search.php";

fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}

/// Local cache mapping addon name -> esoui.com fileinfo page URL.
///
/// Entries are added whenever an addon is successfully resolved (either
/// found in the cache already, or looked up via the esoui search form).
/// If a cached URL later turns out to be invalid (e.g. downloading from it
/// fails), the entry is removed via `invalidate`.
pub struct AddonCache {
    dir: PathBuf,
}

impl AddonCache {
    pub fn new() -> AddonCache {
        let home = dirs::home_dir().expect("failed to determine home directory");
        let dir = home.join(".cache").join(CACHE_DIR_NAME);
        AddonCache { dir }
    }

    /// Construct a cache backed by an arbitrary directory. Mainly useful for
    /// tests, but also allows callers to point the cache elsewhere.
    pub fn new_with_dir(dir: PathBuf) -> AddonCache {
        AddonCache { dir }
    }

    fn addons_path(&self) -> PathBuf {
        self.dir.join(ADDONS_JSON)
    }

    fn ensure_dir(&self) -> Result<()> {
        fs::create_dir_all(&self.dir).map_err(|err| Error::Other(Box::new(err)))
    }

    fn load(&self) -> Result<HashMap<String, String>> {
        match fs::read_to_string(self.addons_path()) {
            Ok(data) => serde_json::from_str(&data).map_err(|err| Error::Other(Box::new(err))),
            Err(_) => Ok(HashMap::new()),
        }
    }

    fn save(&self, map: &HashMap<String, String>) -> Result<()> {
        self.ensure_dir()?;
        let data = serde_json::to_string_pretty(map).map_err(|err| Error::Other(Box::new(err)))?;
        fs::write(self.addons_path(), data).map_err(|err| Error::Other(Box::new(err)))
    }

    /// Look up an addon by exact name in the local cache only.
    pub fn get_cached(&self, name: &str) -> Result<Option<String>> {
        let map = self.load()?;
        Ok(map.get(name).cloned())
    }

    /// Add/update an entry in the cache.
    pub fn insert(&self, name: &str, url: &str) -> Result<()> {
        let mut map = self.load()?;
        map.insert(name.to_owned(), url.to_owned());
        self.save(&map)
    }

    /// Remove a (now invalid) entry from the cache.
    pub fn invalidate(&self, name: &str) -> Result<()> {
        let mut map = self.load()?;
        if map.remove(name).is_some() {
            self.save(&map)?;
        }
        Ok(())
    }

    /// Look up an addon by name, using the local cache first, and falling
    /// back to the esoui.com search form if it isn't cached. A successful
    /// lookup via search is stored back into the cache.
    pub fn find_addon(&self, name: &str) -> Result<Option<String>> {
        if let Some(url) = self.get_cached(name)? {
            return Ok(Some(url));
        }

        match self.search_addon(name)? {
            Some(url) => {
                self.insert(name, &url)?;
                Ok(Some(url))
            }
            None => Ok(None),
        }
    }

    /// Search esoui.com for an addon by (title) name using the classic
    /// search form. If the search resolves directly to a single addon
    /// fileinfo page (as esoui.com does for exact/unique title matches),
    /// that page's URL is returned.
    fn search_addon(&self, name: &str) -> Result<Option<String>> {
        let url = format!(
            "{}?action=search&search={}&titleonly=1",
            SEARCH_URL,
            percent_encode(name)
        );
        let response = reqwest::blocking::get(&url).map_err(|err| Error::Other(Box::new(err)))?;

        let final_url = response.url().to_string();

        if final_url.contains("/downloads/info") || final_url.contains("fileinfo.php") {
            return Ok(Some(final_url));
        }

        // Search results are not always redirected to the matching addon page.
        // In that case, use the first addon fileinfo link in the result page.
        let body = response.text().map_err(|err| Error::Other(Box::new(err)))?;
        let document = Html::parse_document(&body);
        let selector = Selector::parse("a[href]").unwrap();

        for link in document.select(&selector) {
            let href = link.value().attr("href").unwrap_or("");
            if !(href.contains("/downloads/info") || href.contains("fileinfo.php")) {
                continue;
            }

            let absolute_url = if href.starts_with("http://") || href.starts_with("https://") {
                href.to_owned()
            } else if href.starts_with('/') {
                format!("https://www.esoui.com{}", href)
            } else {
                format!("https://www.esoui.com/downloads/{}", href)
            };
            return Ok(Some(absolute_url));
        }

        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_cache() -> (AddonCache, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let cache = AddonCache::new_with_dir(dir.path().to_path_buf());
        (cache, dir)
    }

    #[test]
    fn test_get_cached_missing() {
        let (cache, _dir) = test_cache();
        let result = cache.get_cached("SomeAddon").unwrap();
        assert_eq!(result, None);
    }

    #[test]
    fn test_insert_and_get_cached() {
        let (cache, _dir) = test_cache();
        cache
            .insert(
                "SomeAddon",
                "https://www.esoui.com/downloads/info1-SomeAddon.html",
            )
            .unwrap();

        let result = cache.get_cached("SomeAddon").unwrap();
        assert_eq!(
            result,
            Some("https://www.esoui.com/downloads/info1-SomeAddon.html".to_owned())
        );
    }

    #[test]
    fn test_insert_overwrites_existing_entry() {
        let (cache, _dir) = test_cache();
        cache
            .insert(
                "SomeAddon",
                "https://www.esoui.com/downloads/info1-Old.html",
            )
            .unwrap();
        cache
            .insert(
                "SomeAddon",
                "https://www.esoui.com/downloads/info1-New.html",
            )
            .unwrap();

        let result = cache.get_cached("SomeAddon").unwrap();
        assert_eq!(
            result,
            Some("https://www.esoui.com/downloads/info1-New.html".to_owned())
        );
    }

    #[test]
    fn test_insert_persists_across_instances() {
        let dir = tempfile::tempdir().unwrap();

        let cache = AddonCache::new_with_dir(dir.path().to_path_buf());
        cache
            .insert(
                "SomeAddon",
                "https://www.esoui.com/downloads/info1-SomeAddon.html",
            )
            .unwrap();

        let cache2 = AddonCache::new_with_dir(dir.path().to_path_buf());
        let result = cache2.get_cached("SomeAddon").unwrap();
        assert_eq!(
            result,
            Some("https://www.esoui.com/downloads/info1-SomeAddon.html".to_owned())
        );
    }

    #[test]
    fn test_invalidate_removes_entry() {
        let (cache, _dir) = test_cache();
        cache
            .insert(
                "SomeAddon",
                "https://www.esoui.com/downloads/info1-SomeAddon.html",
            )
            .unwrap();

        cache.invalidate("SomeAddon").unwrap();

        let result = cache.get_cached("SomeAddon").unwrap();
        assert_eq!(result, None);
    }

    #[test]
    fn test_invalidate_missing_entry_is_noop() {
        let (cache, _dir) = test_cache();
        // Should not error even though the entry doesn't exist.
        cache.invalidate("DoesNotExist").unwrap();

        let result = cache.get_cached("DoesNotExist").unwrap();
        assert_eq!(result, None);
    }

    #[test]
    fn test_invalidate_only_removes_named_entry() {
        let (cache, _dir) = test_cache();
        cache
            .insert(
                "AddonA",
                "https://www.esoui.com/downloads/info1-AddonA.html",
            )
            .unwrap();
        cache
            .insert(
                "AddonB",
                "https://www.esoui.com/downloads/info2-AddonB.html",
            )
            .unwrap();

        cache.invalidate("AddonA").unwrap();

        assert_eq!(cache.get_cached("AddonA").unwrap(), None);
        assert_eq!(
            cache.get_cached("AddonB").unwrap(),
            Some("https://www.esoui.com/downloads/info2-AddonB.html".to_owned())
        );
    }

    #[test]
    fn test_find_addon_uses_cache_when_present() {
        let (cache, _dir) = test_cache();
        cache
            .insert(
                "SomeAddon",
                "https://www.esoui.com/downloads/info1-SomeAddon.html",
            )
            .unwrap();

        // Since the addon is already cached, find_addon must not need to
        // reach the network; it should just return the cached value.
        let result = cache.find_addon("SomeAddon").unwrap();
        assert_eq!(
            result,
            Some("https://www.esoui.com/downloads/info1-SomeAddon.html".to_owned())
        );
    }

    #[test]
    fn test_percent_encode() {
        assert_eq!(percent_encode("LibLazyCrafting"), "LibLazyCrafting");
        assert_eq!(percent_encode("Lib Lazy Crafting"), "Lib%20Lazy%20Crafting");
        assert_eq!(percent_encode("a&b"), "a%26b");
    }
}
