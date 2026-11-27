//! The bundled module catalog, embedded at build time from `modules/`
//! (see `build.rs`). Written in-house; nothing is fetched at run time.

use crate::module::{check_manifest, Category, Manifest, Module};
use std::collections::BTreeMap;

pub(crate) struct BundledModule {
    path: &'static str,
    manifest: &'static str,
    files: &'static [(&'static str, &'static str)],
}

include!(concat!(env!("OUT_DIR"), "/bundled.rs"));

#[derive(Debug, Clone, Default)]
pub struct Catalog {
    modules: BTreeMap<String, Module>,
}

impl Catalog {
    /// Parses the bundled catalog.
    ///
    /// Fails with the module path and reason when a manifest is broken; the
    /// catalog test keeps that from ever shipping.
    pub fn bundled() -> Result<Self, String> {
        let mut catalog = Self::default();
        for b in BUNDLED {
            let manifest: Manifest =
                toml::from_str(b.manifest).map_err(|e| format!("{}: {e}", b.path))?;
            let files = b
                .files
                .iter()
                .map(|(name, text)| (name.to_string(), text.replace("\r\n", "\n")))
                .collect();
            catalog.insert(Module {
                manifest,
                path: b.path.to_string(),
                files,
            })?;
        }
        Ok(catalog)
    }

    /// Adds one module after checking its manifest. Ids must be unique.
    pub fn insert(&mut self, module: Module) -> Result<(), String> {
        check_manifest(&module.manifest, &module.files)?;
        let id = module.manifest.id.clone();
        if self.modules.contains_key(&id) {
            return Err(format!("module {id} is defined twice"));
        }
        self.modules.insert(id, module);
        Ok(())
    }

    pub fn get(&self, id: &str) -> Option<&Module> {
        self.modules.get(id)
    }

    pub fn len(&self) -> usize {
        self.modules.len()
    }

    pub fn is_empty(&self) -> bool {
        self.modules.is_empty()
    }

    /// Modules grouped for the palette: by category, then by name.
    pub fn by_category(&self) -> BTreeMap<Category, Vec<&Module>> {
        let mut out: BTreeMap<Category, Vec<&Module>> = BTreeMap::new();
        for m in self.modules.values() {
            out.entry(m.manifest.category).or_default().push(m);
        }
        for list in out.values_mut() {
            list.sort_by(|a, b| a.manifest.name.cmp(&b.manifest.name));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::module::Executor;

    #[test]
    fn the_bundled_catalog_loads() {
        let catalog = Catalog::bundled().expect("bundled catalog");
        assert!(catalog.len() >= 10, "only {} modules", catalog.len());
        for id in [
            "host.ssh",
            "host.this_pc",
            "android.device",
            "automation.custom_script",
        ] {
            assert!(catalog.get(id).is_some(), "{id} missing");
        }
        // Every module sits in the folder its id names.
        for m in catalog.modules.values() {
            let (category, name) = m.manifest.id.split_once('.').unwrap();
            assert_eq!(m.path, format!("{category}/{name}"), "{}", m.manifest.id);
        }
    }

    #[test]
    fn script_files_are_checked() {
        let manifest = |executor| Manifest {
            id: "automation.example".into(),
            name: "Example".into(),
            category: Category::Automation,
            summary: "Example".into(),
            version: 1,
            executor,
            binds: None,
            os: Vec::new(),
            needs: Vec::new(),
            root_switch: None,
            user_scripts: false,
            timeout_secs: 60,
            params: Vec::new(),
            docs_url: None,
        };
        let module = |executor, files: &[(&str, &str)]| Module {
            manifest: manifest(executor),
            path: "automation/example".into(),
            files: files
                .iter()
                .map(|(a, b)| (a.to_string(), b.to_string()))
                .collect(),
        };
        let mut c = Catalog::default();
        assert!(c.insert(module(Executor::SshScript, &[])).is_err());
        assert!(c
            .insert(module(
                Executor::SshScript,
                &[("apply.sh", ""), ("run.sh", "")]
            ))
            .is_err());
        assert!(c
            .insert(module(Executor::Adb, &[("apply.sh", "")]))
            .is_err());
        c.insert(module(
            Executor::SshScript,
            &[("apply.sh", ""), ("check.sh", "")],
        ))
        .unwrap();
        assert!(c
            .insert(module(Executor::SshScript, &[("apply.sh", "")]))
            .is_err());
    }
}
