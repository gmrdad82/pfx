use crate::licence::credit_line;
use crate::{Asset, Download, Kind, Listing, Net, Origin, Plan, Query, Source, Want, check_id};

pub const LICENCE: &str = "CC0-1.0";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pack {
    pub id: &'static str,
    pub name: &'static str,
    pub tags: &'static [&'static str],
    pub page: &'static str,
    pub download: Option<Pinned>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pinned {
    pub url: &'static str,
    pub sha256: &'static str,
    pub size: u64,
}

pub struct Catalog {
    pub name: &'static str,
    pub about: &'static str,
    pub author: &'static str,
    pub why_by_hand: Option<&'static str>,
    pub packs: &'static [Pack],
}

const fn kenney(
    id: &'static str,
    name: &'static str,
    tags: &'static [&'static str],
    page: &'static str,
    url: &'static str,
    sha256: &'static str,
    size: u64,
) -> Pack {
    Pack {
        id,
        name,
        tags,
        page,
        download: Some(Pinned { url, sha256, size }),
    }
}

const KENNEY: &[Pack] = &[
    kenney(
        "car-kit",
        "Car Kit",
        &["vehicle", "car", "road"],
        "https://kenney.nl/assets/car-kit",
        "https://kenney.nl/media/pages/assets/car-kit/1a312ec241-1775131960/kenney_car-kit.zip",
        "fac7dacac5c7874348cf19729af3ef205f3d366493edaf0a827d93f4fdf3d0c4",
        4_814_237,
    ),
    kenney(
        "city-kit-commercial",
        "City Kit (Commercial)",
        &["city", "building", "shop"],
        "https://kenney.nl/assets/city-kit-commercial",
        "https://kenney.nl/media/pages/assets/city-kit-commercial/a742d900eb-1753115042/kenney_city-kit-commercial_2.1.zip",
        "f8b09b081c2bb88bcc126e2dec1cb40fd0dad7e7e591b6c26aaefe96fb35276b",
        4_096_974,
    ),
    kenney(
        "city-kit-roads",
        "City Kit (Roads)",
        &["city", "road", "street"],
        "https://kenney.nl/assets/city-kit-roads",
        "https://kenney.nl/media/pages/assets/city-kit-roads/74288c9459-1791628607/kenney_city-kit-roads.zip",
        "374a4552115cbd11b4ec5c930ee31a533562dc78a09af249a655d03f7ea3b959",
        2_852_289,
    ),
    kenney(
        "city-kit-suburban",
        "City Kit (Suburban)",
        &["city", "house", "building"],
        "https://kenney.nl/assets/city-kit-suburban",
        "https://kenney.nl/media/pages/assets/city-kit-suburban/2c871b7af2-1745479373/kenney_city-kit-suburban_20.zip",
        "5869c35cf30b1c87bdb2d197b6d325eebadd2ef08ea27f04797e8e08d77a9a39",
        3_038_740,
    ),
    kenney(
        "food-kit",
        "Food Kit",
        &["food", "prop"],
        "https://kenney.nl/assets/food-kit",
        "https://kenney.nl/media/pages/assets/food-kit/83086fa91c-1719418518/kenney_food-kit.zip",
        "cdad90853682499b94c9fda2f87678b24bfd8f3264e0ed323f6b6a27fd7c6f6f",
        4_606_270,
    ),
    kenney(
        "furniture-kit",
        "Furniture Kit",
        &["furniture", "interior", "prop"],
        "https://kenney.nl/assets/furniture-kit",
        "https://kenney.nl/media/pages/assets/furniture-kit/440e0608a4-1677580847/kenney_furniture-kit.zip",
        "e67652d0932cee41683f74711c03d3e192a2af9979ef8e6b237711f5482d46b0",
        5_130_729,
    ),
    kenney(
        "nature-kit",
        "Nature Kit",
        &["nature", "tree", "rock", "plant"],
        "https://kenney.nl/assets/nature-kit",
        "https://kenney.nl/media/pages/assets/nature-kit/37ac38a37b-1677698939/kenney_nature-kit.zip",
        "fa7974a0d342bfe63c38664ba9f8ec1a4aab8ea25f099bdc56870e33588c4d9d",
        10_537_521,
    ),
    kenney(
        "platformer-kit",
        "Platformer Kit",
        &["platform", "level", "block"],
        "https://kenney.nl/assets/platformer-kit",
        "https://kenney.nl/media/pages/assets/platformer-kit/1585cf62b4-1775122253/kenney_platformer-kit.zip",
        "899605d237367688c6b42e41fff2206c0fb8d626163158294556353f7baa7c1b",
        4_640_135,
    ),
    kenney(
        "space-kit",
        "Space Kit",
        &["space", "rocket", "station"],
        "https://kenney.nl/assets/space-kit",
        "https://kenney.nl/media/pages/assets/space-kit/20874c75ac-1677698978/kenney_space-kit.zip",
        "d5d7cdf2635ed5a43a9187deaf409b6f47484e402321128341d3c3698e9ef4d9",
        6_677_531,
    ),
];

const fn by_hand(
    id: &'static str,
    name: &'static str,
    tags: &'static [&'static str],
    page: &'static str,
) -> Pack {
    Pack {
        id,
        name,
        tags,
        page,
        download: None,
    }
}

const QUATERNIUS: &[Pack] = &[
    by_hand(
        "buildings",
        "Buildings Pack",
        &["city", "building"],
        "https://quaternius.com/packs/buildings.html",
    ),
    by_hand(
        "cars",
        "Cars Pack",
        &["vehicle", "car"],
        "https://quaternius.com/packs/cars.html",
    ),
    by_hand(
        "medieval-village",
        "Medieval Village Pack",
        &["village", "building", "medieval"],
        "https://quaternius.com/packs/medievalvillage.html",
    ),
    by_hand(
        "stylized-nature-megakit",
        "Stylized Nature MegaKit",
        &["nature", "tree", "plant"],
        "https://quaternius.com/packs/stylizednaturemegakit.html",
    ),
    by_hand(
        "ultimate-furniture",
        "Ultimate Furniture Pack",
        &["furniture", "interior"],
        "https://quaternius.com/packs/ultimatefurniture.html",
    ),
    by_hand(
        "ultimate-modular-ruins",
        "Ultimate Modular Ruins Pack",
        &["ruins", "modular", "stone"],
        "https://quaternius.com/packs/ultimatemodularruins.html",
    ),
    by_hand(
        "ultimate-nature",
        "Ultimate Nature Pack",
        &["nature", "tree", "rock"],
        "https://quaternius.com/packs/ultimatenature.html",
    ),
    by_hand(
        "ultimate-space-kit",
        "Ultimate Space Kit",
        &["space", "ship"],
        "https://quaternius.com/packs/ultimatespacekit.html",
    ),
];

impl Catalog {
    pub fn kenney() -> Catalog {
        Catalog {
            name: "kenney",
            about: "Kenney's CC0 model packs, pinned by URL and sha256",
            author: "Kenney",
            why_by_hand: None,
            packs: KENNEY,
        }
    }

    pub fn quaternius() -> Catalog {
        Catalog {
            name: "quaternius",
            about: "Quaternius's CC0 model packs; downloaded by hand, then taken with --from",
            author: "Quaternius",
            why_by_hand: Some(
                "its packs download from a shared cloud folder that gives no stable file URL or checksum",
            ),
            packs: QUATERNIUS,
        }
    }

    fn listing(&self, pack: &Pack) -> Listing {
        Listing {
            source: self.name.into(),
            id: pack.id.into(),
            name: pack.name.into(),
            kind: Some(Kind::Pack),
            author: Some(self.author.into()),
            licence: Some(LICENCE.into()),
            url: Some(pack.page.into()),
        }
    }
}

fn matches(pack: &Pack, words: &[String]) -> bool {
    let haystack = format!("{} {} {}", pack.id, pack.name, pack.tags.join(" ")).to_lowercase();
    words.iter().all(|w| haystack.contains(w.as_str()))
}

impl Source for Catalog {
    fn name(&self) -> &str {
        self.name
    }

    fn about(&self) -> &str {
        self.about
    }

    fn search(&self, _net: &dyn Net, query: &Query) -> Result<Vec<Listing>, String> {
        if query.kind.is_some_and(|k| k != Kind::Pack) {
            return Ok(Vec::new());
        }
        let words: Vec<String> = query
            .text
            .split_whitespace()
            .map(str::to_lowercase)
            .collect();
        let limit = if query.limit == 0 {
            usize::MAX
        } else {
            query.limit
        };
        Ok(self
            .packs
            .iter()
            .filter(|p| matches(p, &words))
            .take(limit)
            .map(|p| self.listing(p))
            .collect())
    }

    fn plan(&self, _net: &dyn Net, id: &str, want: &Want) -> Result<Plan, String> {
        check_id(id)?;
        let pack = self.packs.iter().find(|p| p.id == id).ok_or_else(|| {
            format!(
                "{}:{id}: not in the catalog; `pfx asset search --source {}` lists it",
                self.name, self.name
            )
        })?;
        let mut download = match (&want.from, pack.download) {
            (Some(file), pinned) => {
                let mut d = Download {
                    origin: Origin::File(file.clone()),
                    path: format!("{id}.zip"),
                    size: None,
                    md5: None,
                    sha256: None,
                    unpack: true,
                    headers: Vec::new(),
                };
                if let Some(pin) = pinned {
                    d.sha256 = Some(pin.sha256.into());
                    d.size = Some(pin.size);
                }
                d
            }
            (None, Some(pin)) => {
                let mut d = Download::url(pin.url, format!("{id}.zip"));
                d.sha256 = Some(pin.sha256.into());
                d.size = Some(pin.size);
                d
            }
            (None, None) => {
                return Err(format!(
                    "{}:{id} can't be downloaded by pfx: {}. Download it from {} and run `pfx asset get {}:{id} --from <file.zip>`",
                    self.name,
                    self.why_by_hand.unwrap_or("it has no pinned download"),
                    pack.page,
                    self.name
                ));
            }
        };
        download.unpack = true;
        Ok(Plan {
            asset: Asset {
                source: self.name.into(),
                id: id.into(),
                name: pack.name.into(),
                kind: Kind::Pack,
                author: Some(self.author.into()),
                licence: LICENCE.into(),
                credit: credit_line(pack.name, Some(self.author), Some(pack.page), LICENCE),
                url: Some(pack.page.into()),
            },
            downloads: vec![download],
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mock::Mock;

    #[test]
    fn every_pinned_pack_is_https_with_a_sha256_and_ids_are_unique() {
        for catalog in [Catalog::kenney(), Catalog::quaternius()] {
            let mut ids: Vec<&str> = catalog.packs.iter().map(|p| p.id).collect();
            ids.sort_unstable();
            ids.dedup();
            assert_eq!(ids.len(), catalog.packs.len());
            for pack in catalog.packs {
                assert!(check_id(pack.id).is_ok());
                assert!(pack.page.starts_with("https://"));
                if let Some(pin) = pack.download {
                    assert!(pin.url.starts_with("https://"));
                    assert_eq!(pin.sha256.len(), 64);
                    assert!(pin.sha256.bytes().all(|b| b.is_ascii_hexdigit()));
                    assert!(pin.size > 0);
                }
            }
        }
    }

    #[test]
    fn search_matches_every_word_against_names_and_tags() {
        let net = Mock::default();
        let kenney = Catalog::kenney();
        let all = kenney.search(&net, &Query::default()).unwrap();
        assert_eq!(all.len(), KENNEY.len());
        let city = Query {
            text: "City road".into(),
            ..Query::default()
        };
        let found = kenney.search(&net, &city).unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].id, "city-kit-roads");
        let hdri = Query {
            kind: Some(Kind::Hdri),
            ..Query::default()
        };
        assert!(kenney.search(&net, &hdri).unwrap().is_empty());
    }

    #[test]
    fn a_pinned_pack_downloads_by_url_and_a_hand_pack_needs_from() {
        let net = Mock::default();
        let plan = Catalog::kenney()
            .plan(&net, "car-kit", &Want::default())
            .unwrap();
        assert!(plan.downloads[0].unpack);
        assert!(plan.downloads[0].sha256.is_some());
        assert_eq!(
            plan.asset.credit,
            "\"Car Kit\" by Kenney (https://kenney.nl/assets/car-kit) is licensed under CC0 1.0"
        );
        let refused = Catalog::quaternius()
            .plan(&net, "ultimate-nature", &Want::default())
            .unwrap_err();
        assert!(refused.contains("--from"), "{refused}");
        assert!(refused.contains("https://quaternius.com/packs/ultimatenature.html"));
        let from = Want {
            from: Some("pack.zip".into()),
            ..Want::default()
        };
        let plan = Catalog::quaternius()
            .plan(&net, "ultimate-nature", &from)
            .unwrap();
        assert_eq!(plan.downloads[0].origin, Origin::File("pack.zip".into()));
        assert!(plan.downloads[0].sha256.is_none());
        let kenney_from = Catalog::kenney().plan(&net, "car-kit", &from).unwrap();
        assert!(kenney_from.downloads[0].sha256.is_some());
        assert!(
            Catalog::kenney()
                .plan(&net, "no-such-kit", &Want::default())
                .is_err()
        );
    }
}
