use pfx_core::memory::{self, Census, Room, Run};

pub use pfx_core::memory::room;

const GIB: f64 = (1u64 << 30) as f64;

pub fn refusal(view: &str, triangles: Census, texels: u64, room: Room) -> Option<String> {
    let plan = memory::plan(&triangles, Run::Plate { texels });
    let need = plan.cpu();
    if need <= room.free {
        return None;
    }
    let image = texels.saturating_mul(memory::PLATE_PER_TEXEL);
    let share = room.free.saturating_sub(image) as f64 / need.saturating_sub(image).max(1) as f64;
    let free = match room.limit {
        Some(limit) => format!(
            "{:.1} GiB is free under the job's {:.1} GiB memory limit",
            room.free as f64 / GIB,
            limit as f64 / GIB
        ),
        None => format!("{:.1} GiB of memory is free", room.free as f64 / GIB),
    };
    Some(format!(
        "plate {view}: {:.1} M static triangles ({:.1} M in distinct meshes) need about {:.1} GiB to bake and {free}; lower them to about {:.1} M or free memory",
        triangles.placed as f64 / 1e6,
        triangles.distinct as f64 / 1e6,
        need as f64 / GIB,
        triangles.placed as f64 * share / 1e6,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plate_over_the_room_is_refused_in_one_line_naming_the_limit_and_what_to_lower() {
        let room = Room {
            free: 4 << 30,
            limit: Some(5 << 30),
        };
        let unique = |millions: f64| Census {
            placed: (millions * 1e6) as u64,
            distinct: (millions * 1e6) as u64,
            ..Census::default()
        };
        assert_eq!(refusal("main", unique(4.4), 230_400, room), None);
        let repeated = Census {
            placed: 10_035_200,
            distinct: 100_352,
            ..Census::default()
        };
        assert_eq!(refusal("main", repeated, 230_400, room), None);
        let line = refusal("main", unique(10.0), 230_400, room).unwrap();
        assert!(!line.contains('\n'));
        assert!(
            line.starts_with(
                "plate main: 10.0 M static triangles (10.0 M in distinct meshes) need about 5.9 GiB"
            ),
            "{line}"
        );
        assert!(
            line.contains("4.0 GiB is free under the job's 5.0 GiB memory limit"),
            "{line}"
        );
        assert!(
            line.ends_with("lower them to about 6.8 M or free memory"),
            "{line}"
        );
        let open = refusal(
            "main",
            unique(10.0),
            0,
            Room {
                free: 2 << 30,
                limit: None,
            },
        )
        .unwrap();
        assert!(open.contains("2.0 GiB of memory is free"), "{open}");
    }
}
