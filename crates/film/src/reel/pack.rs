const SHELF_SPLIT: u32 = 8;
const ITEM_SPLIT: u32 = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Item {
    x: u32,
    width: u32,
    taken: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Shelf {
    y: u32,
    height: u32,
    empty: bool,
    items: Vec<Item>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Shelves {
    side: u32,
    shelves: Vec<Shelf>,
}

fn shelf_height(size: u32, side: u32) -> u32 {
    let step = match size {
        0..=31 => 8,
        32..=127 => 16,
        128..=511 => 32,
        _ => 64,
    };
    let rem = size % step;
    if rem == 0 {
        return size;
    }
    let up = size + step - rem;
    if up > side { size } else { up }
}

impl Shelves {
    pub fn new(side: u32) -> Shelves {
        Shelves {
            side,
            shelves: vec![Shelf {
                y: 0,
                height: side,
                empty: true,
                items: vec![Item {
                    x: 0,
                    width: side,
                    taken: false,
                }],
            }],
        }
    }

    pub fn allocate(&mut self, size: [u32; 2]) -> Option<[u32; 2]> {
        let [width, height] = size;
        if width == 0 || height == 0 || width > self.side || height > self.side {
            return None;
        }
        let height = shelf_height(height, self.side);
        let bad_fit_threshold = height.saturating_add(height / 2);
        let mut chosen_height = u32::MAX;
        let mut chosen: Option<(usize, usize)> = None;
        for (s, shelf) in self.shelves.iter().enumerate() {
            if shelf.height < height || shelf.height >= chosen_height {
                continue;
            }
            let bad_fit = !shelf.empty && shelf.height > bad_fit_threshold;
            if bad_fit && chosen.is_some() {
                continue;
            }
            let Some(i) = shelf
                .items
                .iter()
                .position(|item| !item.taken && item.width >= width)
            else {
                continue;
            };
            chosen = Some((s, i));
            if !bad_fit {
                chosen_height = shelf.height;
            }
            if shelf.height == height {
                break;
            }
        }
        let (s, i) = chosen?;
        let shelf = self.shelves[s].clone();
        if shelf.empty && shelf.height > height + SHELF_SPLIT {
            self.shelves.insert(
                s + 1,
                Shelf {
                    y: shelf.y + height,
                    height: shelf.height - height,
                    empty: true,
                    items: vec![Item {
                        x: 0,
                        width: self.side,
                        taken: false,
                    }],
                },
            );
            self.shelves[s].height = height;
        }
        let row = &mut self.shelves[s];
        row.empty = false;
        let item = row.items[i];
        if item.width - width > ITEM_SPLIT {
            row.items.insert(
                i + 1,
                Item {
                    x: item.x + width,
                    width: item.width - width,
                    taken: false,
                },
            );
            row.items[i].width = width;
        }
        row.items[i].taken = true;
        Some([item.x, row.y])
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Placed {
    pub page: usize,
    pub at: [u32; 2],
}

pub fn pack(sizes: &[[u32; 2]], side: u32, most_pages: usize) -> Result<Vec<Placed>, usize> {
    let mut order: Vec<usize> = (0..sizes.len()).collect();
    order.sort_by_key(|&i| {
        (
            std::cmp::Reverse(sizes[i][1]),
            std::cmp::Reverse(sizes[i][0]),
            i,
        )
    });
    let mut pages: Vec<Shelves> = Vec::new();
    let mut out = vec![
        Placed {
            page: 0,
            at: [0, 0]
        };
        sizes.len()
    ];
    for i in order {
        let found = pages
            .iter_mut()
            .enumerate()
            .find_map(|(p, page)| page.allocate(sizes[i]).map(|at| Placed { page: p, at }));
        out[i] = match found {
            Some(placed) => placed,
            None if pages.len() < most_pages => {
                let mut page = Shelves::new(side);
                let at = page.allocate(sizes[i]).ok_or(i)?;
                pages.push(page);
                Placed {
                    page: pages.len() - 1,
                    at,
                }
            }
            None => return Err(i),
        };
    }
    Ok(out)
}
