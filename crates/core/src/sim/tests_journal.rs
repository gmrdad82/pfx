use std::time::Instant;

use super::journal::{Digest, Field, Journal, JournalError, JournalWriter, Schema};
use super::{Rng, STREAM_VERSION};

const SIM_VERSION: u32 = 3;

fn every_kind() -> Schema {
    Schema::new(&[
        Field::Flag,
        Field::Unsigned(1),
        Field::Unsigned(12),
        Field::Unsigned(63),
        Field::Signed(1),
        Field::Signed(8),
        Field::Signed(64),
        Field::Flag,
    ])
    .unwrap()
}

fn random_input(rng: &mut Rng, schema: &Schema, previous: &[i64]) -> Vec<i64> {
    schema
        .fields()
        .iter()
        .zip(previous)
        .map(|(field, &old)| match rng.below(10) {
            0..=5 => old,
            6 => field.min(),
            7 => field.max(),
            _ => {
                let span = (field.max() as i128 - field.min() as i128 + 1) as u128;
                (field.min() as i128 + (rng.next_u64() as u128 % span) as i128) as i64
            }
        })
        .collect()
}

fn step(state: &mut u64, input: &[i64]) -> u64 {
    let mut digest = Digest::new();
    digest.u64(*state);
    for &value in input {
        digest.i64(value);
    }
    *state = digest.value();
    *state
}

fn record(schema: &Schema, inputs: &[Vec<i64>], every: u32) -> Vec<u8> {
    let mut writer = JournalWriter::new(schema, 77, SIM_VERSION, every);
    let mut state = 77;
    for input in inputs {
        let digest = step(&mut state, input);
        writer.push(input, || digest).unwrap();
    }
    writer.finish()
}

fn read_all(journal: &Journal<'_>) -> Result<Vec<Vec<i64>>, JournalError> {
    let mut reader = journal.reader();
    let mut out = Vec::new();
    while let Some(input) = reader.read()? {
        out.push(input.to_vec());
    }
    Ok(out)
}

fn scripted(rng: &mut Rng, schema: &Schema, ticks: usize) -> Vec<Vec<i64>> {
    let mut previous = vec![0; schema.fields().len()];
    let mut inputs = Vec::with_capacity(ticks);
    while inputs.len() < ticks {
        let next = random_input(rng, schema, &previous);
        let hold = if rng.below(4) == 0 {
            rng.below(40) as usize
        } else {
            1
        };
        for _ in 0..hold.min(ticks - inputs.len()).max(1) {
            inputs.push(next.clone());
        }
        previous = next;
    }
    inputs
}

#[test]
fn every_field_kind_round_trips_with_and_without_checkpoints() {
    let schema = every_kind();
    let mut rng = Rng::new(5);
    for every in [0, 1, 2, 7, 64, 5000] {
        for ticks in [0, 1, 6, 7, 8, 500] {
            let inputs = scripted(&mut rng, &schema, ticks);
            let bytes = record(&schema, &inputs, every);
            let journal = Journal::open(&bytes).unwrap();
            assert_eq!(journal.header().ticks, ticks as u64);
            assert_eq!(journal.header().stream_version, STREAM_VERSION);
            assert_eq!(journal.schema(), &schema);
            assert_eq!(read_all(&journal).unwrap(), inputs);
            for tick in [0, ticks / 3, ticks / 2, ticks] {
                let mut reader = journal.seek(tick as u64).unwrap();
                let mut rest = Vec::new();
                while let Some(input) = reader.read().unwrap() {
                    rest.push(input.to_vec());
                }
                assert_eq!(rest, inputs[tick..]);
            }
            assert_eq!(
                journal.seek(ticks as u64 + 1).unwrap_err(),
                JournalError::Beyond {
                    tick: ticks as u64 + 1,
                    ticks: ticks as u64
                }
            );
        }
    }
}

#[test]
fn bytes_are_frozen() {
    let schema = Schema::new(&[Field::Unsigned(12), Field::Signed(8), Field::Flag]).unwrap();
    let inputs = [
        [0, 0, 0],
        [0, 0, 0],
        [0, 0, 0],
        [100, -3, 1],
        [101, -3, 1],
        [101, -3, 0],
        [101, -3, 0],
    ];
    let mut writer = JournalWriter::new(&schema, 0x0102_0304_0506_0708, 9, 4);
    for (tick, input) in inputs.iter().enumerate() {
        writer.push(input, || 0xd1d1_0000 + tick as u64).unwrap();
    }
    let hex: String = writer
        .finish()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    assert_eq!(
        hex,
        concat!(
            "5046584a",
            "0100",
            "0300",
            "0807060504030201",
            "09000000",
            "01000000",
            "805330000586075a",
            "0700000000000000",
            "04000000",
            "01000000",
            "0b00000000000000",
            "4b8700",
            "04",
            "0fc80105",
            "0fca0105",
            "09",
            "00",
            "0400000000000000",
            "0300d1d100000000",
            "0500000000000000",
            "74d8cfcc438c5867",
        )
    );
}

#[test]
fn idle_ticks_cost_a_byte_or_less() {
    let schema = Schema::new(&[Field::Signed(16), Field::Signed(16), Field::Flag]).unwrap();
    let idle = vec![vec![5, -5, 1]; 1000];
    let bytes = record(&schema, &idle, 0);
    let journal = Journal::open(&bytes).unwrap();
    assert!(bytes.len() < 56 + 3 + 8 + 8, "{} bytes", bytes.len());
    assert_eq!(read_all(&journal).unwrap(), idle);
    let bytes = record(&schema, &idle, 1);
    assert!(
        bytes.len() - 1000 * 24 < 56 + 3 + 8 + 8 * 1000,
        "{} bytes",
        bytes.len()
    );
}

#[test]
fn writer_refuses_values_outside_their_fields() {
    let schema = Schema::new(&[Field::Unsigned(4), Field::Signed(4), Field::Flag]).unwrap();
    let mut writer = JournalWriter::new(&schema, 0, 0, 0);
    writer.push(&[15, -8, 1], || 0).unwrap();
    writer.push(&[0, 7, 0], || 0).unwrap();
    for (input, field) in [
        ([16, 0, 0], 0),
        ([-1, 0, 0], 0),
        ([0, 8, 0], 1),
        ([0, -9, 0], 1),
        ([0, 0, 2], 2),
    ] {
        assert_eq!(
            writer.push(&input, || 0),
            Err(JournalError::Input { tick: 2, field })
        );
    }
    assert_eq!(
        writer.push(&[0, 0], || 0),
        Err(JournalError::InputLength {
            found: 2,
            expected: 3
        })
    );
    assert_eq!(writer.ticks(), 2);
    let bytes = writer.finish();
    assert_eq!(
        read_all(&Journal::open(&bytes).unwrap()).unwrap(),
        [[15, -8, 1], [0, 7, 0]]
    );
}

#[test]
fn schemas_outside_the_limits_are_refused() {
    for fields in [
        vec![],
        vec![Field::Unsigned(0)],
        vec![Field::Unsigned(64)],
        vec![Field::Signed(0)],
        vec![Field::Signed(65)],
        vec![Field::Flag; 64],
    ] {
        assert_eq!(Schema::new(&fields), Err(JournalError::Schema));
    }
    assert!(Schema::new(&[Field::Flag; 63]).is_ok());
    assert_eq!(Field::Signed(64).min(), i64::MIN);
    assert_eq!(Field::Unsigned(63).max(), i64::MAX);
    assert_eq!((Field::Signed(1).min(), Field::Signed(1).max()), (-1, 0));
}

#[test]
fn verify_replays_and_fails_at_the_first_checkpoint_that_disagrees() {
    let schema = every_kind();
    let mut rng = Rng::new(11);
    let inputs = scripted(&mut rng, &schema, 300);
    let bytes = record(&schema, &inputs, 32);
    let journal = Journal::open(&bytes).unwrap();
    let mut state = 77;
    let mut expected = None;
    for input in &inputs {
        expected = Some(step(&mut state, input));
    }
    let mut state = 77;
    assert_eq!(
        journal.verify(&schema, SIM_VERSION, |input| step(&mut state, input)),
        Ok(expected)
    );

    let mut state = 77;
    let mut ticks = 0;
    let diverged = journal.verify(&schema, SIM_VERSION, |input| {
        ticks += 1;
        if ticks == 50 {
            state ^= 1;
        }
        step(&mut state, input)
    });
    assert!(matches!(
        diverged,
        Err(JournalError::Checkpoint { tick: 64, .. })
    ));
    assert_eq!(ticks, 64);

    let empty = record(&schema, &[], 32);
    assert_eq!(
        Journal::open(&empty)
            .unwrap()
            .verify(&schema, SIM_VERSION, |_| 0),
        Ok(None)
    );
}

#[test]
fn a_writer_hands_out_its_bytes_and_verify_asks_only_for_the_digests_it_checks() {
    let schema = every_kind();
    let mut rng = Rng::new(12);
    let inputs = scripted(&mut rng, &schema, 301);
    let mut writer = JournalWriter::new(&schema, 77, SIM_VERSION, 32);
    let mut state = 77;
    let mut flushed = Vec::new();
    for (tick, input) in inputs.iter().enumerate() {
        let digest = step(&mut state, input);
        writer.push(input, || digest).unwrap();
        if tick % 60 == 59 {
            writer.bytes_into(&mut flushed);
            assert_eq!(flushed, record(&schema, &inputs[..=tick], 32));
        }
    }
    let bytes = writer.bytes();
    let reused = writer.snapshot_into(vec![9; 3]);
    assert_eq!(writer.snapshot().seal(), bytes);
    assert_eq!(reused.seal(), bytes);
    assert_eq!(writer.finish(), bytes);
    let journal = Journal::open(&bytes).unwrap();
    let mut state = 77;
    let mut asked = Vec::new();
    let mut tick = 0;
    let last = journal.verify_sparse(&schema, SIM_VERSION, |input, wanted| {
        tick += 1;
        let digest = step(&mut state, input);
        if wanted {
            asked.push(tick);
        }
        digest
    });
    assert_eq!(last, Ok(Some(state)));
    let mut expected: Vec<u64> = (1..=9).map(|k| k * 32).collect();
    expected.push(301);
    assert_eq!(asked, expected);
}

#[test]
fn mismatched_versions_and_schemas_are_refused() {
    let schema = every_kind();
    let bytes = record(&schema, &[vec![0; 8]], 0);
    let journal = Journal::open(&bytes).unwrap();
    assert_eq!(
        journal.verify(&schema, SIM_VERSION + 1, |_| 0),
        Err(JournalError::SimVersion {
            found: SIM_VERSION,
            expected: SIM_VERSION + 1
        })
    );
    let other = Schema::new(&[Field::Flag]).unwrap();
    assert!(matches!(
        journal.verify(&other, SIM_VERSION, |_| 0),
        Err(JournalError::SchemaMismatch { .. })
    ));

    let mut bytes = bytes;
    bytes[20] = 9;
    reseal(&mut bytes);
    assert_eq!(
        Journal::open(&bytes).unwrap().check(&schema, SIM_VERSION),
        Err(JournalError::StreamVersion { found: 9 })
    );
    bytes[4] = 2;
    assert_eq!(
        Journal::open(&bytes).unwrap_err(),
        JournalError::Format { found: 2 }
    );
    bytes[0] = b'X';
    assert_eq!(Journal::open(&bytes).unwrap_err(), JournalError::Magic);
}

fn reseal(bytes: &mut [u8]) {
    let end = bytes.len() - 8;
    let sum = Digest::new().bytes(&bytes[..end]).value();
    bytes[end..].copy_from_slice(&sum.to_le_bytes());
}

#[test]
fn truncated_or_damaged_journals_are_errors_never_panics() {
    let schema = every_kind();
    let mut rng = Rng::new(23);
    let inputs = scripted(&mut rng, &schema, 120);
    let bytes = record(&schema, &inputs, 16);
    for len in 0..bytes.len() {
        assert!(Journal::open(&bytes[..len]).is_err(), "length {len}");
    }
    let mut longer = bytes.clone();
    longer.push(0);
    assert!(Journal::open(&longer).is_err());
    for at in 0..bytes.len() {
        let mut flipped = bytes.clone();
        flipped[at] ^= 0x10;
        assert!(Journal::open(&flipped).is_err(), "byte {at}");
    }
    for at in 0..bytes.len() - 8 {
        for value in [0x00, 0x01, 0x7f, 0x80, 0xff, rng.next_u32() as u8] {
            let mut damaged = bytes.clone();
            damaged[at] = value;
            reseal(&mut damaged);
            if let Ok(journal) = Journal::open(&damaged) {
                let _ = read_all(&journal);
                let _ = journal.verify(&schema, SIM_VERSION, |_| 0);
                let _ = journal
                    .seek(journal.header().ticks / 2)
                    .map(|mut reader| reader.read().map(|v| v.map(<[i64]>::to_vec)));
            }
        }
    }
}

#[test]
fn a_stream_with_one_encoding_per_input() {
    let schema = Schema::new(&[Field::Unsigned(8)]).unwrap();
    let mut bytes = record(&schema, &[vec![1], vec![1]], 0);
    let stream = 56 + 1;
    assert_eq!(&bytes[stream..stream + 3], [0x03, 0x02, 0x00]);
    for (patch, at) in [
        ([0x03, 0x00, 0x00], stream + 1),
        ([0x03, 0x82, 0x00], stream + 1),
        ([0x05, 0x02, 0x00], stream),
        ([0x04, 0x02, 0x00], stream),
    ] {
        bytes[stream..stream + 3].copy_from_slice(&patch);
        reseal(&mut bytes);
        assert_eq!(
            read_all(&Journal::open(&bytes).unwrap()),
            Err(JournalError::Corrupt { at: at as u64 }),
            "{patch:02x?}"
        );
    }
}

#[test]
#[ignore = "bench; run with --release"]
fn bench_twenty_five_minutes_of_pointer_and_buttons() {
    let schema = Schema::new(&[
        Field::Signed(16),
        Field::Signed(16),
        Field::Flag,
        Field::Flag,
        Field::Flag,
        Field::Flag,
    ])
    .unwrap();
    let ticks = 25 * 60 * 60;
    let mut rng = Rng::new(60);
    let mut input = [1920i64, 1080, 0, 0, 0, 0];
    let mut velocity = [0i64, 0];
    let inputs: Vec<[i64; 6]> = (0..ticks)
        .map(|_| {
            for axis in 0..2 {
                velocity[axis] = (velocity[axis] + rng.range_i64(-3, 4)).clamp(-24, 24);
                let size = [3840, 2160][axis];
                let next = input[axis] + velocity[axis];
                if !(0..size).contains(&next) {
                    velocity[axis] = -velocity[axis];
                }
                input[axis] = next.clamp(0, size - 1);
            }
            for button in &mut input[2..] {
                if rng.below(90) == 0 {
                    *button ^= 1;
                }
            }
            input
        })
        .collect();
    let digest = Digest::new().value();
    let mut best_write = f64::MAX;
    let mut best_read = f64::MAX;
    let mut best_snapshot = f64::MAX;
    let mut best_bytes = f64::MAX;
    let mut bytes = Vec::new();
    let mut buffer = Vec::new();
    for _ in 0..5 {
        let start = Instant::now();
        let mut writer = JournalWriter::new(&schema, 1, 1, 600);
        for input in &inputs {
            writer.push(input, || digest).unwrap();
        }
        let wrote = start.elapsed().as_secs_f64();
        let start = Instant::now();
        let unsealed = writer.snapshot_into(std::mem::take(&mut buffer));
        best_snapshot = best_snapshot.min(start.elapsed().as_secs_f64());
        let start = Instant::now();
        writer.bytes_into(&mut buffer);
        best_bytes = best_bytes.min(start.elapsed().as_secs_f64());
        assert_eq!(unsealed.seal(), buffer);
        let start = Instant::now();
        bytes = writer.finish();
        best_write = best_write.min(wrote + start.elapsed().as_secs_f64());
        best_write = best_write.min(start.elapsed().as_secs_f64());
        let start = Instant::now();
        let journal = Journal::open(&bytes).unwrap();
        let mut reader = journal.reader();
        let mut sum = 0i64;
        while let Some(input) = reader.read().unwrap() {
            sum = sum.wrapping_add(input[0]);
        }
        best_read = best_read.min(start.elapsed().as_secs_f64());
        assert_ne!(sum, 0);
    }
    println!(
        "{ticks} ticks: {} bytes, {:.2} bytes/tick; write {:.3} µs/tick, read {:.3} µs/tick",
        bytes.len(),
        bytes.len() as f64 / ticks as f64,
        best_write * 1e6 / ticks as f64,
        best_read * 1e6 / ticks as f64,
    );
    println!(
        "at the end: snapshot {:.1} µs, bytes {:.1} µs",
        best_snapshot * 1e6,
        best_bytes * 1e6
    );
}
