//! Sequential safety boundary, independent of UI and native APIs.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Situation {
    Stopped,
    Running,
    TargetRunning,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum QuitProgress {
    Running,
    Waiting,
    Exited,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Stage {
    Waiting,
    Switching,
}

pub(super) trait Actions {
    type Error;
    fn inspect(&mut self) -> Result<Situation, Self::Error>;
    fn request_quit(&mut self) -> Result<(), Self::Error>;
    fn poll_quit(&mut self) -> Result<QuitProgress, Self::Error>;
    fn finish(&mut self) -> Result<(), Self::Error>;
    fn activate(&mut self) -> Result<(), Self::Error>;
}

pub(super) fn run<A: Actions>(
    actions: &mut A,
    mut stage: impl FnMut(Stage),
    mut wait: impl FnMut(),
) -> Result<(), A::Error> {
    match actions.inspect()? {
        Situation::TargetRunning => return actions.activate(),
        Situation::Stopped => {}
        Situation::Running => {
            actions.request_quit()?;
            stage(Stage::Waiting);
            loop {
                match actions.poll_quit()? {
                    QuitProgress::Exited => break,
                    QuitProgress::Running | QuitProgress::Waiting => wait(),
                }
            }
        }
    }
    stage(Stage::Switching);
    actions.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    struct Fake {
        situation: Result<Situation, &'static str>,
        request: Result<(), &'static str>,
        polls: VecDeque<Result<QuitProgress, &'static str>>,
        events: Vec<&'static str>,
    }
    impl Fake {
        fn running(polls: Vec<Result<QuitProgress, &'static str>>) -> Self {
            Self {
                situation: Ok(Situation::Running),
                request: Ok(()),
                polls: polls.into(),
                events: vec![],
            }
        }
    }
    impl Actions for Fake {
        type Error = &'static str;
        fn inspect(&mut self) -> Result<Situation, Self::Error> {
            self.events.push("inspect");
            self.situation
        }
        fn request_quit(&mut self) -> Result<(), Self::Error> {
            self.events.push("request");
            self.request
        }
        fn poll_quit(&mut self) -> Result<QuitProgress, Self::Error> {
            self.events.push("poll");
            self.polls.pop_front().expect("no exit was observed")
        }
        fn finish(&mut self) -> Result<(), Self::Error> {
            self.events.push("sync-and-launch");
            Ok(())
        }
        fn activate(&mut self) -> Result<(), Self::Error> {
            self.events.push("activate");
            Ok(())
        }
    }

    #[test]
    fn busy_and_input_wait_never_sync_or_reissue_quit() {
        let mut a = Fake::running(vec![
            Ok(QuitProgress::Running),
            Ok(QuitProgress::Waiting),
            Ok(QuitProgress::Waiting),
            Ok(QuitProgress::Exited),
        ]);
        let mut stages = vec![];
        let mut waits = 0;
        run(&mut a, |s| stages.push(s), || waits += 1).unwrap();
        assert_eq!(
            a.events,
            [
                "inspect",
                "request",
                "poll",
                "poll",
                "poll",
                "poll",
                "sync-and-launch"
            ]
        );
        assert_eq!(stages, [Stage::Waiting, Stage::Switching]);
        assert_eq!(waits, 3);
    }

    #[test]
    fn unknown_dialog_or_changed_identity_leaves_stores_untouched() {
        for error in ["unknown_dialog", "identity_changed", "permission_revoked"] {
            let mut a = Fake::running(vec![Ok(QuitProgress::Waiting), Err(error)]);
            assert_eq!(run(&mut a, |_| {}, || {}), Err(error));
            assert_eq!(a.events, ["inspect", "request", "poll", "poll"]);
        }
    }

    #[test]
    fn refused_inspection_or_quit_never_polls_or_switches() {
        let mut a = Fake::running(vec![]);
        a.situation = Err("ambiguous_processes");
        assert_eq!(run(&mut a, |_| {}, || {}), Err("ambiguous_processes"));
        assert_eq!(a.events, ["inspect"]);
        let mut a = Fake::running(vec![]);
        a.request = Err("unsupported_build");
        assert_eq!(run(&mut a, |_| {}, || {}), Err("unsupported_build"));
        assert_eq!(a.events, ["inspect", "request"]);
    }

    #[test]
    fn stopped_launches_directly_and_existing_target_only_activates() {
        for (situation, expected) in [
            (Situation::Stopped, vec!["inspect", "sync-and-launch"]),
            (Situation::TargetRunning, vec!["inspect", "activate"]),
        ] {
            let mut a = Fake::running(vec![]);
            a.situation = Ok(situation);
            run(&mut a, |_| {}, || panic!("must not wait")).unwrap();
            assert_eq!(a.events, expected);
        }
    }
}
