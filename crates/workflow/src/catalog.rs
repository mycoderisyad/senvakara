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
            guard: None,
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

    #[test]
    fn server_modules_name_their_systems_and_risky_ones_are_guarded() {
        let catalog = Catalog::bundled().unwrap();
        for m in catalog.modules.values() {
            let category = m.manifest.category;
            if !matches!(category, Category::Security | Category::Networking) {
                continue;
            }
            let id = &m.manifest.id;
            assert_eq!(m.manifest.os, ["debian>=12", "ubuntu>=22.04"], "{id}");
            assert!(m.files.contains_key("check.sh"), "{id} has no check");
            assert!(
                m.manifest.needs.contains(&crate::Permission::Root),
                "{id} runs as root"
            );
            assert!(m.manifest.docs_url.is_some(), "{id} names its docs");
        }
        for id in ["security.ssh_hardening", "networking.firewall"] {
            let m = catalog.get(id).unwrap();
            assert!(m.manifest.guard.is_some(), "{id} is guarded");
            assert!(m.files.contains_key("rollback.sh"), "{id} rolls back");
        }
        let ssh = catalog.get("security.ssh_hardening").unwrap();
        assert_eq!(ssh.manifest.os_text(), "Debian 12+, Ubuntu 22.04+");
        assert_eq!(
            ssh.manifest.guard.as_ref().unwrap().port_param.as_deref(),
            Some("port")
        );
    }

    #[test]
    fn guards_need_their_rollback_and_permissions() {
        let base = || {
            let mut m = catalog_module("networking.example");
            m.manifest.guard = Some(crate::Guard {
                rollback_secs: 180,
                port_param: None,
            });
            m
        };
        let mut c = Catalog::default();
        let mut no_rollback = base();
        no_rollback.files.remove("rollback.sh");
        assert!(c.insert(no_rollback).is_err());
        let mut no_network = base();
        no_network.manifest.needs = vec![crate::Permission::Root];
        assert!(c.insert(no_network).is_err());
        let mut too_long = base();
        too_long.manifest.guard.as_mut().unwrap().rollback_secs = 3600;
        assert!(c.insert(too_long).is_err());
        let mut bad_port = base();
        bad_port.manifest.guard.as_mut().unwrap().port_param = Some("missing".into());
        assert!(c.insert(bad_port).is_err());
        let mut stray = catalog_module("networking.stray");
        stray.manifest.guard = None;
        assert!(c.insert(stray).is_err(), "rollback.sh without a guard");
        c.insert(base()).unwrap();
    }

    #[test]
    fn engine_names_cannot_be_parameters() {
        let mut m = catalog_module("networking.example");
        m.files.remove("rollback.sh");
        m.manifest.params = vec![crate::ParamSpec {
            name: "session_user".into(),
            label: "User".into(),
            kind: crate::ParamKind::Text,
            required: false,
            default: None,
            min: None,
            max: None,
            options: Vec::new(),
            max_len: None,
            suffix: None,
            help: None,
        }];
        assert!(Catalog::default().insert(m).is_err());
    }

    #[test]
    fn os_release_gives_id_and_version() {
        let text = "PRETTY_NAME=\"Ubuntu 24.04.1 LTS\"\nNAME=\"Ubuntu\"\n\
                    VERSION_ID=\"24.04\"\nID=ubuntu\nID_LIKE=debian\n";
        assert_eq!(
            crate::parse_os_release(text),
            Some(("ubuntu".into(), "24.04".into()))
        );
        assert_eq!(crate::parse_os_release("ID=debian\n"), None);
    }

    /// A guarded ssh_script module with every file it needs.
    fn catalog_module(id: &str) -> Module {
        let manifest = Manifest {
            id: id.into(),
            name: "Example".into(),
            category: Category::Networking,
            summary: "Example".into(),
            version: 1,
            executor: Executor::SshScript,
            binds: None,
            os: Vec::new(),
            needs: vec![crate::Permission::Root, crate::Permission::Network],
            root_switch: None,
            user_scripts: false,
            timeout_secs: 60,
            params: Vec::new(),
            docs_url: None,
            guard: None,
        };
        Module {
            manifest,
            path: id.replace('.', "/"),
            files: ["check.sh", "apply.sh", "rollback.sh"]
                .iter()
                .map(|f| (f.to_string(), String::new()))
                .collect(),
        }
    }
}
