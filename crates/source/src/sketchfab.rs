use serde_json::Value;

use crate::licence::{EDITORIAL, credit_line};
use crate::secret::Credential;
use crate::{
    Asset, Download, Kind, Listing, Net, Plan, Query, Source, Want, check_id, encode, text_of,
};

pub const API: &str = "https://api.sketchfab.com/v3";

const SLUGS: [(&str, &str); 9] = [
    ("cc0", "CC0-1.0"),
    ("by", "CC-BY-4.0"),
    ("by-sa", "CC-BY-SA-4.0"),
    ("by-nd", "CC-BY-ND-4.0"),
    ("by-nc", "CC-BY-NC-4.0"),
    ("by-nc-sa", "CC-BY-NC-SA-4.0"),
    ("by-nc-nd", "CC-BY-NC-ND-4.0"),
    ("ed", EDITORIAL),
    ("st", "LicenseRef-Sketchfab-Standard"),
];

const LABELS: [(&str, &str); 9] = [
    ("CC0 Public Domain", "CC0-1.0"),
    ("CC Attribution", "CC-BY-4.0"),
    ("CC Attribution-ShareAlike", "CC-BY-SA-4.0"),
    ("CC Attribution-NoDerivs", "CC-BY-ND-4.0"),
    ("CC Attribution-NonCommercial", "CC-BY-NC-4.0"),
    ("CC Attribution-NonCommercial-ShareAlike", "CC-BY-NC-SA-4.0"),
    ("CC Attribution-NonCommercial-NoDerivs", "CC-BY-NC-ND-4.0"),
    ("Editorial", EDITORIAL),
    ("Standard", "LicenseRef-Sketchfab-Standard"),
];

pub struct Sketchfab {
    pub name: String,
    pub credential: Credential,
}

pub fn spdx(licence: &Value) -> Option<String> {
    let by = |table: &[(&str, &str)], key: &str| {
        let found = text_of(licence, key)?;
        table
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(&found))
            .map(|(_, id)| id.to_string())
    };
    by(&SLUGS, "slug").or_else(|| by(&LABELS, "label"))
}

fn author(user: Option<&Value>) -> Option<String> {
    let user = user?;
    text_of(user, "displayName").or_else(|| text_of(user, "username"))
}

impl Source for Sketchfab {
    fn name(&self) -> &str {
        &self.name
    }

    fn about(&self) -> &str {
        "Sketchfab's downloadable models; the account's API token is read at run time"
    }

    fn search(&self, net: &dyn Net, query: &Query) -> Result<Vec<Listing>, String> {
        if query.kind.is_some_and(|k| k != Kind::Model) {
            return Ok(Vec::new());
        }
        let limit = query.limit.clamp(1, 24);
        let url = format!(
            "{API}/search?type=models&downloadable=true&count={limit}&q={}",
            encode(query.text.trim())
        );
        let found = net.json(&url, &Vec::new())?;
        let results = found
            .get("results")
            .and_then(Value::as_array)
            .ok_or("Sketchfab's search gave no results list")?;
        Ok(results
            .iter()
            .filter_map(|model| {
                let id = text_of(model, "uid").filter(|id| check_id(id).is_ok())?;
                Some(Listing {
                    source: self.name.clone(),
                    name: text_of(model, "name").unwrap_or_else(|| id.clone()),
                    kind: Some(Kind::Model),
                    author: author(model.get("user")),
                    licence: model.get("license").and_then(spdx),
                    url: text_of(model, "viewerUrl"),
                    id,
                })
            })
            .take(limit)
            .collect())
    }

    fn plan(&self, net: &dyn Net, id: &str, want: &Want) -> Result<Plan, String> {
        check_id(id)?;
        let reference = format!("{}:{id}", self.name);
        if want.from.is_some() {
            return Err(format!("{reference}: --from is for catalog packs"));
        }
        let model = net.json(&format!("{API}/models/{id}"), &Vec::new())?;
        let licence = model
            .get("license")
            .and_then(spdx)
            .ok_or_else(|| format!("{reference}: Sketchfab gives no licence pfx knows"))?;
        let name = text_of(&model, "name").unwrap_or_else(|| id.to_string());
        let by = author(model.get("user"));
        let page = text_of(&model, "viewerUrl");
        want.policy
            .check(&licence)
            .map_err(|e| format!("{reference}: {e}"))?;
        let token = self
            .credential
            .resolve()
            .map_err(|e| format!("{reference}: {e}"))?;
        let headers = vec![(
            "Authorization".to_string(),
            format!("Token {}", token.expose()),
        )];
        let links = net.json(&format!("{API}/models/{id}/download"), &headers)?;
        let format = want.format.clone().unwrap_or_else(|| "glb".into());
        let link = links
            .get(&format)
            .or_else(|| (want.format.is_none()).then(|| links.get("gltf")).flatten())
            .ok_or_else(|| {
                format!("{reference}: no {format} download, Sketchfab offers other formats")
            })?;
        let url = text_of(link, "url")
            .filter(|u| u.starts_with("https://"))
            .ok_or_else(|| format!("{reference}: the download has no https url"))?;
        let zipped = !crate::shown(&url).ends_with(".glb");
        let mut download = Download::url(
            url,
            if zipped {
                format!("{id}.zip")
            } else {
                format!("{id}.glb")
            },
        );
        download.size = link.get("size").and_then(Value::as_u64);
        download.unpack = zipped;
        Ok(Plan {
            asset: Asset {
                source: self.name.clone(),
                id: id.into(),
                credit: credit_line(&name, by.as_deref(), page.as_deref(), &licence),
                name,
                kind: Kind::Model,
                author: by,
                licence,
                url: page,
            },
            downloads: vec![download],
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mock::Mock;

    const ID: &str = "0123456789abcdef0123456789abcdef";

    fn net() -> Mock {
        Mock::default()
            .with(
                &format!("{API}/models/{ID}"),
                format!(r#"{{"uid":"{ID}","name":"Lamp","viewerUrl":"https://sketchfab.com/3d-models/lamp-{ID}","user":{{"displayName":"E. Author"}},"license":{{"slug":"by","label":"CC Attribution"}}}}"#),
            )
            .with(
                &format!("{API}/models/{ID}/download"),
                r#"{"glb":{"url":"https://media.example.org/lamp.glb?X-Signature=abc","size":12},"gltf":{"url":"https://media.example.org/lamp.zip?sig=1","size":20}}"#,
            )
            .with(
                &format!("{API}/search?type=models&downloadable=true&count=2&q=lamp"),
                format!(r#"{{"results":[{{"uid":"{ID}","name":"Lamp","user":{{"username":"e"}},"license":{{"label":"CC Attribution-NonCommercial"}}}}]}}"#),
            )
    }

    #[test]
    fn licences_map_to_spdx_by_slug_or_label() {
        assert_eq!(
            spdx(&serde_json::json!({"slug": "by-nc-nd"})).as_deref(),
            Some("CC-BY-NC-ND-4.0")
        );
        assert_eq!(
            spdx(&serde_json::json!({"label": "Editorial"})).as_deref(),
            Some(EDITORIAL)
        );
        assert_eq!(spdx(&serde_json::json!({"label": "Something else"})), None);
        for (_, id) in SLUGS.iter().chain(LABELS.iter()) {
            assert!(pfx_core::spdx::validate(id).is_ok(), "{id}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn the_token_goes_only_on_the_download_request() {
        let net = net();
        let source = Sketchfab {
            name: "models".into(),
            credential: Credential::Command(vec!["printf".into(), "tok-9".into()]),
        };
        let plan = source.plan(&net, ID, &Want::default()).unwrap();
        assert_eq!(plan.asset.licence, "CC-BY-4.0");
        assert_eq!(
            plan.asset.credit,
            format!(
                "\"Lamp\" by E. Author (https://sketchfab.com/3d-models/lamp-{ID}) is licensed under CC BY 4.0"
            )
        );
        assert_eq!(plan.downloads[0].path, format!("{ID}.glb"));
        assert!(!plan.downloads[0].unpack);
        assert!(plan.downloads[0].headers.is_empty());
        for (url, headers) in net.asked.borrow().iter() {
            let carries = headers.iter().any(|(_, v)| v.contains("tok-9"));
            assert_eq!(carries, url.ends_with("/download"), "{url}");
        }
        let gltf = Want {
            format: Some("gltf".into()),
            ..Want::default()
        };
        let plan = source.plan(&net, ID, &gltf).unwrap();
        assert!(plan.downloads[0].unpack);
    }

    #[test]
    fn a_missing_token_is_refused_before_any_download_link() {
        let net = net();
        let source = Sketchfab {
            name: "models".into(),
            credential: Credential::Env("PFX_TEST_UNSET_MODELS_TOKEN".into()),
        };
        let error = source.plan(&net, ID, &Want::default()).unwrap_err();
        assert!(error.contains("PFX_TEST_UNSET_MODELS_TOKEN"), "{error}");
        assert!(
            net.asked
                .borrow()
                .iter()
                .all(|(url, _)| !url.ends_with("/download"))
        );
    }

    #[test]
    fn a_refused_licence_asks_for_no_token() {
        let net = Mock::default().with(
            &format!("{API}/models/{ID}"),
            format!(r#"{{"uid":"{ID}","name":"Lamp","license":{{"slug":"by-nc"}}}}"#),
        );
        let source = Sketchfab {
            name: "models".into(),
            credential: Credential::Env("PFX_TEST_UNSET_MODELS_TOKEN".into()),
        };
        let error = source.plan(&net, ID, &Want::default()).unwrap_err();
        assert!(error.contains("--allow nc"), "{error}");
        assert_eq!(net.asked.borrow().len(), 1);
    }

    #[test]
    fn search_lists_downloadable_models_with_their_licence() {
        let source = Sketchfab {
            name: "models".into(),
            credential: Credential::Env("UNUSED".into()),
        };
        let query = Query {
            text: "lamp".into(),
            kind: None,
            limit: 2,
        };
        let net = net();
        let found = source.search(&net, &query).unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].licence.as_deref(), Some("CC-BY-NC-4.0"));
        assert_eq!(found[0].author.as_deref(), Some("e"));
        assert!(
            net.asked
                .borrow()
                .iter()
                .all(|(_, headers)| headers.is_empty())
        );
    }
}
