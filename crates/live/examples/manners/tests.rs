use crate::manners::*;

mod jobs {
    use super::*;
    use std::cell::RefCell;

    struct Watch;

    impl Sink for Watch {
        fn progress(&mut self, _done: u64, _total: u64, _fps: f64) {}

        fn end(&mut self, _ok: bool) {}
    }

    fn env<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        move |key| {
            pairs
                .iter()
                .find(|(name, _)| *name == key)
                .map(|(_, value)| value.to_string())
        }
    }

    #[test]
    fn a_record_starts_only_under_pgpu() {
        let started = RefCell::new(Vec::new());
        let start = |label: &str, pgpu: u32| -> Box<dyn Sink> {
            started.borrow_mut().push((label.to_string(), pgpu));
            Box::new(Watch)
        };
        Tally::timed_in(env(&[]), "bench", "moving", 600, 0.0, start);
        Tally::timed_in(
            env(&[("GPU_QUEUE_PID", "none")]),
            "bench",
            "moving",
            600,
            0.0,
            start,
        );
        assert!(started.borrow().is_empty());
        Tally::timed_in(
            env(&[("GPU_QUEUE_PID", "4242")]),
            "desk",
            "breakdown",
            600,
            0.0,
            start,
        );
        assert_eq!(*started.borrow(), [("desk breakdown".to_string(), 4242)]);
    }
}
