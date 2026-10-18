use serde_json::Value;

use crate::{
    Asset, Download, Kind, Listing, Net, Plan, Query, Source, Want, check_id, encode, text_of,
};

pub const API: &str = "https://ambientcg.com/api/v3/assets";
pub const SITE: &str = "https://ambientcg.com/a/";
pub const LICENCE: &str = "CC0-1.0";
pub const AUTHOR: &str = "ambientCG";

pub struct AmbientCg;

fn kind_of(word: &str) -> Kind {
    match word {
        "hdri" | "hdri-element" => Kind::Hdri,
        "3d-model" => Kind::Model,
        _ => Kind::Texture,
    }
}

fn type_words(kind: Kind) -> Result<&'static str, String> {
    match kind {
        Kind::Hdri => Ok("hdri"),
        Kind::Texture => Ok("material,decal,atlas,plain-image,terrain"),
        Kind::Model => Ok("3d-model"),
        Kind::Pack => Err("ambientCG has no packs; ask for hdri, texture or model".into()),
    }
}

pub fn credit(title: &str) -> String {
    format!(
        "Created using {title} from ambientCG.com, licensed under the Creative Commons CC0 1.0 Universal License."
    )
}

fn choose<'a>(downloads: &'a [Value], want: &Want) -> Result<&'a Value, String> {
    let attribute = |d: &Value| text_of(d, "attributes").unwrap_or_default();
    let resolution = want.resolution.clone().unwrap_or_else(|| "1K".into());
    let format = want.format.clone();
    let fits = |d: &&Value| {
        let words: Vec<String> = attribute(d)
            .split('-')
            .map(str::to_ascii_uppercase)
            .collect();
        words.contains(&resolution.to_ascii_uppercase())
            && format
                .as_ref()
                .is_none_or(|f| words.contains(&f.to_ascii_uppercase()))
    };
    let found: Vec<&Value> = downloads.iter().filter(fits).collect();
    let preferred = found
        .iter()
        .find(|d| attribute(d).to_ascii_uppercase().ends_with("-JPG"))
        .or_else(|| found.first());
    preferred.copied().ok_or_else(|| {
        let have: Vec<String> = downloads.iter().map(attribute).collect();
        format!(
            "no download matches {resolution}{}, it has: {}",
            format.map(|f| format!(" {f}")).unwrap_or_default(),
            have.join(", ")
        )
    })
}

impl Source for AmbientCg {
    fn name(&self) -> &str {
        "ambientcg"
    }

    fn about(&self) -> &str {
        "ambientCG's CC0 materials, HDRIs and models, through its API v3"
    }

    fn search(&self, net: &dyn Net, query: &Query) -> Result<Vec<Listing>, String> {
        let limit = query.limit.clamp(1, 100);
        let mut url = format!(
            "{API}?q={}&limit={limit}&include=title,type,url",
            encode(query.text.trim())
        );
        if let Some(kind) = query.kind {
            url.push_str(&format!("&type={}", type_words(kind)?));
        }
        let found = net.json(&url, &Vec::new())?;
        let assets = found
            .get("assets")
            .and_then(Value::as_array)
            .ok_or("ambientCG's search gave no assets list")?;
        Ok(assets
            .iter()
            .filter_map(|asset| {
                let id = text_of(asset, "id").filter(|id| check_id(id).is_ok())?;
                Some(Listing {
                    source: self.name().into(),
                    name: text_of(asset, "title").unwrap_or_else(|| id.clone()),
                    kind: text_of(asset, "type").map(|t| kind_of(&t)),
                    author: Some(AUTHOR.into()),
                    licence: Some(LICENCE.into()),
                    url: Some(format!("{SITE}{id}")),
                    id,
                })
            })
            .take(limit)
            .collect())
    }

    fn plan(&self, net: &dyn Net, id: &str, want: &Want) -> Result<Plan, String> {
        check_id(id)?;
        if want.from.is_some() {
            return Err(
                "ambientcg assets download by themselves; --from is for catalog packs".into(),
            );
        }
        let found = net.json(
            &format!("{API}?id={id}&include=title,type,url,downloads"),
            &Vec::new(),
        )?;
        let asset = found
            .get("assets")
            .and_then(Value::as_array)
            .and_then(|list| {
                list.iter()
                    .find(|a| text_of(a, "id").as_deref() == Some(id))
            })
            .ok_or_else(|| format!("ambientcg:{id}: no such asset"))?;
        let downloads = asset
            .get("downloads")
            .and_then(Value::as_array)
            .ok_or_else(|| format!("ambientcg:{id}: the asset lists no downloads"))?;
        let chosen = choose(downloads, want).map_err(|e| format!("ambientcg:{id}: {e}"))?;
        let url = text_of(chosen, "url")
            .filter(|u| u.starts_with("https://"))
            .ok_or_else(|| format!("ambientcg:{id}: the download has no https url"))?;
        let attribute = text_of(chosen, "attributes").unwrap_or_else(|| "file".into());
        let extension = text_of(chosen, "extension").unwrap_or_else(|| "zip".into());
        let mut download = Download::url(url, format!("{id}_{attribute}.{extension}"));
        download.size = chosen.get("size").and_then(Value::as_u64);
        download.unpack = extension.eq_ignore_ascii_case("zip");
        let title = text_of(asset, "title").unwrap_or_else(|| id.to_string());
        Ok(Plan {
            asset: Asset {
                source: self.name().into(),
                id: id.into(),
                credit: credit(&title),
                name: title,
                kind: text_of(asset, "type").map_or(Kind::Texture, |t| kind_of(&t)),
                author: Some(AUTHOR.into()),
                licence: LICENCE.into(),
                url: Some(format!("{SITE}{id}")),
            },
            downloads: vec![download],
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mock::Mock;

    const ASSET: &str = r#"{"assets":[{"id":"Stone001","title":"Stone 001","type":"material","url":"https://ambientcg.com/a/Stone001","downloads":[
        {"attributes":"1K-JPG","extension":"zip","url":"https://ambientcg.com/get?file=Stone001_1K-JPG.zip","size":100},
        {"attributes":"1K-PNG","extension":"zip","url":"https://ambientcg.com/get?file=Stone001_1K-PNG.zip","size":300},
        {"attributes":"2K-JPG","extension":"zip","url":"https://ambientcg.com/get?file=Stone001_2K-JPG.zip","size":400}]}]}"#;

    fn net() -> Mock {
        Mock::default()
            .with(&format!("{API}?id=Stone001&include=title,type,url,downloads"), ASSET)
            .with(
                &format!("{API}?q=stone&limit=3&include=title,type,url&type=hdri"),
                r#"{"totalResults":1,"assets":[{"id":"Sky002","title":"Sky 002","type":"hdri"},{"id":"bad/id"}]}"#,
            )
    }

    #[test]
    fn a_plan_picks_the_1k_jpg_zip_and_unpacks_it() {
        let plan = AmbientCg
            .plan(&net(), "Stone001", &Want::default())
            .unwrap();
        assert_eq!(plan.downloads.len(), 1);
        let download = &plan.downloads[0];
        assert!(download.unpack);
        assert_eq!(download.size, Some(100));
        assert_eq!(download.path, "Stone001_1K-JPG.zip");
        assert_eq!(plan.asset.licence, "CC0-1.0");
        assert_eq!(
            plan.asset.credit,
            "Created using Stone 001 from ambientCG.com, licensed under the Creative Commons CC0 1.0 Universal License."
        );
        let png = Want {
            format: Some("png".into()),
            ..Want::default()
        };
        assert_eq!(
            AmbientCg.plan(&net(), "Stone001", &png).unwrap().downloads[0].path,
            "Stone001_1K-PNG.zip"
        );
        let big = Want {
            resolution: Some("8K".into()),
            ..Want::default()
        };
        let error = AmbientCg.plan(&net(), "Stone001", &big).unwrap_err();
        assert!(error.contains("1K-JPG, 1K-PNG, 2K-JPG"), "{error}");
    }

    #[test]
    fn search_keeps_only_well_formed_ids() {
        let query = Query {
            text: "stone".into(),
            kind: Some(Kind::Hdri),
            limit: 3,
        };
        let found = AmbientCg.search(&net(), &query).unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].kind, Some(Kind::Hdri));
        assert_eq!(
            found[0].url.as_deref(),
            Some("https://ambientcg.com/a/Sky002")
        );
    }
}
