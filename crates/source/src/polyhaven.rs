use serde_json::Value;

use crate::licence::credit_line;
use crate::{
    Asset, Download, Kind, Listing, Net, Plan, Query, Source, Want, check_id, encode, text_of,
};

pub const API: &str = "https://api.polyhaven.com";
pub const SITE: &str = "https://polyhaven.com/a/";
pub const LICENCE: &str = "CC0-1.0";
const MAP_FORMATS: [&str; 3] = ["jpg", "png", "exr"];
const BUNDLES: [&str; 6] = ["blend", "gltf", "mtlx", "usd", "fbx", "tonemapped"];

pub struct PolyHaven;

fn kind_of(value: &Value) -> Option<Kind> {
    match value.get("type").and_then(Value::as_u64) {
        Some(0) => Some(Kind::Hdri),
        Some(1) => Some(Kind::Texture),
        Some(2) => Some(Kind::Model),
        _ => None,
    }
}

fn type_word(kind: Kind) -> Result<&'static str, String> {
    match kind {
        Kind::Hdri => Ok("hdris"),
        Kind::Texture => Ok("textures"),
        Kind::Model => Ok("models"),
        Kind::Pack => Err("Poly Haven has no packs; ask for hdri, texture or model".into()),
    }
}

fn authors(info: &Value) -> Option<String> {
    let list: Vec<&str> = info
        .get("authors")
        .and_then(Value::as_object)?
        .keys()
        .map(String::as_str)
        .collect();
    (!list.is_empty()).then(|| list.join(", "))
}

fn file(value: &Value, path: &str) -> Result<Download, String> {
    let url = text_of(value, "url").ok_or_else(|| format!("{path}: the file list gives no url"))?;
    if !url.starts_with("https://") {
        return Err(format!("{path}: {url} is not an https URL"));
    }
    let mut download = Download::url(url, path);
    download.size = value.get("size").and_then(Value::as_u64);
    download.md5 = text_of(value, "md5");
    Ok(download)
}

fn base_name(url: &str) -> String {
    crate::shown(url)
        .rsplit('/')
        .next()
        .unwrap_or("file")
        .to_string()
}

fn pick<'a>(
    table: &'a Value,
    what: &str,
    resolution: &str,
) -> Result<&'a serde_json::Map<String, Value>, String> {
    let by_res = table
        .as_object()
        .ok_or_else(|| format!("{what}: the file list is not a table"))?;
    by_res
        .get(resolution)
        .and_then(Value::as_object)
        .ok_or_else(|| {
            let mut have: Vec<&str> = by_res.keys().map(String::as_str).collect();
            have.sort_by_key(|r| r.trim_end_matches('k').parse::<u32>().unwrap_or(u32::MAX));
            format!("{what} has no {resolution}, it has: {}", have.join(", "))
        })
}

fn downloads(kind: Kind, files: &Value, want: &Want) -> Result<Vec<Download>, String> {
    let resolution = want.resolution.clone().unwrap_or_else(|| "1k".into());
    let mut out = Vec::new();
    match kind {
        Kind::Hdri => {
            let format = want.format.clone().unwrap_or_else(|| "hdr".into());
            let table = files.get("hdri").ok_or("the file list has no hdri")?;
            let formats = pick(table, "the hdri", &resolution)?;
            let chosen = formats.get(&format).ok_or_else(|| {
                format!(
                    "the hdri has no {format} at {resolution}, it has: {}",
                    formats.keys().cloned().collect::<Vec<_>>().join(", ")
                )
            })?;
            let url = text_of(chosen, "url").unwrap_or_default();
            out.push(file(chosen, &base_name(&url))?);
        }
        Kind::Texture => {
            let format = want.format.clone().unwrap_or_else(|| "jpg".into());
            if !MAP_FORMATS.contains(&format.as_str()) {
                return Err(format!(
                    "textures come as {}, not {format}",
                    MAP_FORMATS.join(", ")
                ));
            }
            let maps = files.as_object().ok_or("the file list is not a table")?;
            for (map, table) in maps {
                if BUNDLES.contains(&map.as_str()) {
                    continue;
                }
                let formats = pick(table, &format!("the {map} map"), &resolution)?;
                if let Some(chosen) = formats.get(&format) {
                    let url = text_of(chosen, "url").unwrap_or_default();
                    out.push(file(chosen, &base_name(&url))?);
                }
            }
            if out.is_empty() {
                return Err(format!("no map comes as {format} at {resolution}"));
            }
        }
        Kind::Model => {
            let table = files.get("gltf").ok_or("the model has no glTF files")?;
            let formats = pick(table, "the model", &resolution)?;
            let gltf = formats.get("gltf").ok_or("the model has no .gltf")?;
            let url = text_of(gltf, "url").unwrap_or_default();
            out.push(file(gltf, &base_name(&url))?);
            if let Some(include) = gltf.get("include").and_then(Value::as_object) {
                for (path, value) in include {
                    out.push(file(value, path)?);
                }
            }
        }
        Kind::Pack => return Err("Poly Haven has no packs".into()),
    }
    Ok(out)
}

impl Source for PolyHaven {
    fn name(&self) -> &str {
        "polyhaven"
    }

    fn about(&self) -> &str {
        "Poly Haven's CC0 HDRIs, textures and models, through its public API"
    }

    fn powered_by(&self) -> Option<&'static str> {
        Some("Powered by Poly Haven")
    }

    fn search(&self, net: &dyn Net, query: &Query) -> Result<Vec<Listing>, String> {
        let limit = query.limit.clamp(1, 50);
        let mut url = format!(
            "{API}/search?q={}&limit={limit}",
            encode(&query.text.trim().to_lowercase())
        );
        if let Some(kind) = query.kind {
            url.push_str(&format!("&t={}", type_word(kind)?));
        }
        let found = net.json(&url, &Vec::new())?;
        let results = found
            .get("results")
            .and_then(Value::as_array)
            .ok_or("Poly Haven's search gave no results list")?;
        let mut out = Vec::new();
        for result in results.iter().take(limit) {
            let Some(id) = text_of(result, "slug") else {
                continue;
            };
            if check_id(&id).is_err() {
                continue;
            }
            let info = net.json(&format!("{API}/info/{id}"), &Vec::new())?;
            out.push(Listing {
                source: self.name().into(),
                name: text_of(&info, "name").unwrap_or_else(|| id.clone()),
                kind: kind_of(&info),
                author: authors(&info),
                licence: Some(LICENCE.into()),
                url: Some(format!("{SITE}{id}")),
                id,
            });
        }
        Ok(out)
    }

    fn plan(&self, net: &dyn Net, id: &str, want: &Want) -> Result<Plan, String> {
        check_id(id)?;
        if want.from.is_some() {
            return Err(
                "polyhaven assets download by themselves; --from is for catalog packs".into(),
            );
        }
        let info = net.json(&format!("{API}/info/{id}"), &Vec::new())?;
        let kind = kind_of(&info).ok_or_else(|| format!("polyhaven:{id}: no such asset"))?;
        let files = net.json(&format!("{API}/files/{id}"), &Vec::new())?;
        let name = text_of(&info, "name").unwrap_or_else(|| id.to_string());
        let author = authors(&info);
        let url = format!("{SITE}{id}");
        Ok(Plan {
            asset: Asset {
                source: self.name().into(),
                id: id.into(),
                credit: credit_line(&name, author.as_deref(), Some(&url), LICENCE),
                name,
                kind,
                author,
                licence: LICENCE.into(),
                url: Some(url),
            },
            downloads: downloads(kind, &files, want).map_err(|e| format!("polyhaven:{id}: {e}"))?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mock::Mock;

    const INFO_HDRI: &str = r#"{"type":0,"name":"Field Sky","authors":{"A. Author":"All"}}"#;
    const FILES_HDRI: &str = r#"{"hdri":{"1k":{"hdr":{"url":"https://dl.example.org/hdr/1k/field_sky_1k.hdr","size":10,"md5":"0123456789abcdef0123456789abcdef"},"exr":{"url":"https://dl.example.org/exr/1k/field_sky_1k.exr","size":9,"md5":"ff"}},"2k":{}},"tonemapped":{"url":"https://dl.example.org/t.jpg"}}"#;
    const INFO_MODEL: &str =
        r#"{"type":2,"name":"Crate","authors":{"B. Author":"Model","C. Author":"Textures"}}"#;
    const FILES_MODEL: &str = r#"{"gltf":{"1k":{"gltf":{"url":"https://dl.example.org/crate_1k.gltf","size":5,"md5":"aa","include":{"crate.bin":{"url":"https://dl.example.org/crate.bin","size":4,"md5":"bb"},"textures/crate_diff_1k.jpg":{"url":"https://dl.example.org/crate_diff_1k.jpg","size":3,"md5":"cc"}}}}},"blend":{}}"#;
    const INFO_TEXTURE: &str = r#"{"type":1,"name":"Brick","authors":{"D. Author":"All"}}"#;
    const FILES_TEXTURE: &str = r#"{"Diffuse":{"1k":{"jpg":{"url":"https://dl.example.org/brick_diff_1k.jpg","size":1,"md5":"a"},"png":{"url":"https://dl.example.org/brick_diff_1k.png","size":1,"md5":"b"}}},"nor_gl":{"1k":{"jpg":{"url":"https://dl.example.org/brick_nor_gl_1k.jpg","size":1,"md5":"c"}}},"gltf":{"1k":{}},"blend":{"1k":{}}}"#;

    fn net() -> Mock {
        Mock::default()
            .with(&format!("{API}/info/field_sky"), INFO_HDRI)
            .with(&format!("{API}/files/field_sky"), FILES_HDRI)
            .with(&format!("{API}/info/crate"), INFO_MODEL)
            .with(&format!("{API}/files/crate"), FILES_MODEL)
            .with(&format!("{API}/info/brick"), INFO_TEXTURE)
            .with(&format!("{API}/files/brick"), FILES_TEXTURE)
            .with(
                &format!("{API}/search?q=field%20sky&limit=5&t=hdris"),
                r#"{"total":1,"results":[{"slug":"field_sky","score":0.7},{"slug":"../bad","score":0.1}]}"#,
            )
    }

    #[test]
    fn search_lists_names_authors_and_cc0() {
        let net = net();
        let query = Query {
            text: " Field Sky ".into(),
            kind: Some(Kind::Hdri),
            limit: 5,
        };
        let found = PolyHaven.search(&net, &query).unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].name, "Field Sky");
        assert_eq!(found[0].author.as_deref(), Some("A. Author"));
        assert_eq!(found[0].licence.as_deref(), Some("CC0-1.0"));
        assert_eq!(
            found[0].url.as_deref(),
            Some("https://polyhaven.com/a/field_sky")
        );
        assert_eq!(PolyHaven.powered_by(), Some("Powered by Poly Haven"));
    }

    #[test]
    fn an_hdri_plan_takes_the_asked_resolution_and_format_with_its_checksum() {
        let plan = PolyHaven
            .plan(&net(), "field_sky", &Want::default())
            .unwrap();
        assert_eq!(plan.asset.kind, Kind::Hdri);
        assert_eq!(plan.asset.licence, "CC0-1.0");
        assert_eq!(
            plan.asset.credit,
            "\"Field Sky\" by A. Author (https://polyhaven.com/a/field_sky) is licensed under CC0 1.0"
        );
        assert_eq!(plan.downloads.len(), 1);
        assert_eq!(plan.downloads[0].path, "field_sky_1k.hdr");
        assert_eq!(plan.downloads[0].size, Some(10));
        assert!(plan.downloads[0].md5.is_some());
        let exr = Want {
            format: Some("exr".into()),
            ..Want::default()
        };
        assert_eq!(
            PolyHaven.plan(&net(), "field_sky", &exr).unwrap().downloads[0].path,
            "field_sky_1k.exr"
        );
        let missing = Want {
            resolution: Some("8k".into()),
            ..Want::default()
        };
        let error = PolyHaven.plan(&net(), "field_sky", &missing).unwrap_err();
        assert!(error.contains("1k, 2k"), "{error}");
    }

    #[test]
    fn a_model_brings_its_gltf_and_every_include_at_its_path() {
        let plan = PolyHaven.plan(&net(), "crate", &Want::default()).unwrap();
        let paths: Vec<&str> = plan.downloads.iter().map(|d| d.path.as_str()).collect();
        assert_eq!(
            paths,
            ["crate_1k.gltf", "crate.bin", "textures/crate_diff_1k.jpg"]
        );
        assert_eq!(plan.asset.author.as_deref(), Some("B. Author, C. Author"));
    }

    #[test]
    fn a_texture_brings_each_map_in_the_asked_format() {
        let plan = PolyHaven.plan(&net(), "brick", &Want::default()).unwrap();
        let mut paths: Vec<&str> = plan.downloads.iter().map(|d| d.path.as_str()).collect();
        paths.sort_unstable();
        assert_eq!(paths, ["brick_diff_1k.jpg", "brick_nor_gl_1k.jpg"]);
        assert!(PolyHaven.plan(&net(), "../x", &Want::default()).is_err());
    }
}
