use crate::output::style::{Palette, Role, Terminal};
use indicatif::{ProgressBar, ProgressDrawTarget, ProgressStyle};
use std::{future::Future, time::Duration};

struct Spinner(ProgressBar);

impl Spinner {
    fn start(target: ProgressDrawTarget, palette: Palette) -> Self {
        Self::labelled(target, palette, "Получение треда…")
    }

    fn labelled(target: ProgressDrawTarget, palette: Palette, label: &str) -> Self {
        let bar = ProgressBar::with_draw_target(None, target);
        bar.set_style(
            ProgressStyle::default_spinner()
                .tick_strings(&["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"]),
        );
        // Название операции доверенное: запрос или ожидание следующего опроса.
        let template = format!("{{spinner}} {}", palette.paint(Role::Loading, label));
        if let Ok(style) = bar.style().template(&template) {
            bar.set_style(style);
        }
        if !bar.is_hidden() {
            bar.enable_steady_tick(Duration::from_millis(100));
            bar.tick();
        }
        Self(bar)
    }
}

impl Drop for Spinner {
    fn drop(&mut self) {
        self.0.disable_steady_tick();
        self.0.finish_and_clear();
    }
}

pub async fn fetching<T>(
    terminal: Terminal,
    palette: Palette,
    request: impl Future<Output = T>,
) -> T {
    let target = if terminal.animation() {
        ProgressDrawTarget::stderr()
    } else {
        ProgressDrawTarget::hidden()
    };
    with_target(target, palette, request).await
}

pub async fn waiting<T>(terminal: Terminal, palette: Palette, wait: impl Future<Output = T>) -> T {
    let target = if terminal.animation() {
        ProgressDrawTarget::stderr()
    } else {
        ProgressDrawTarget::hidden()
    };
    let _spinner = Spinner::labelled(target, palette, "Ожидание новых анкет… Ctrl+C — остановка");
    wait.await
}

async fn with_target<T>(
    target: ProgressDrawTarget,
    palette: Palette,
    request: impl Future<Output = T>,
) -> T {
    let _spinner = Spinner::start(target, palette);
    request.await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::output::style::{ColorDepth, Theme};
    use indicatif::TermLike;
    use std::{
        io,
        sync::{Arc, Mutex},
    };

    #[derive(Debug, Default)]
    struct Screen {
        writes: Vec<String>,
        visible: String,
        clears: usize,
    }

    #[derive(Clone, Debug, Default)]
    struct TestTerm(Arc<Mutex<Screen>>);

    impl TermLike for TestTerm {
        fn width(&self) -> u16 {
            80
        }
        fn move_cursor_up(&self, _: usize) -> io::Result<()> {
            Ok(())
        }
        fn move_cursor_down(&self, _: usize) -> io::Result<()> {
            Ok(())
        }
        fn move_cursor_right(&self, _: usize) -> io::Result<()> {
            Ok(())
        }
        fn move_cursor_left(&self, _: usize) -> io::Result<()> {
            Ok(())
        }
        fn write_line(&self, s: &str) -> io::Result<()> {
            self.write_str(s)
        }
        fn write_str(&self, s: &str) -> io::Result<()> {
            let mut screen = self.0.lock().unwrap();
            screen.writes.push(s.to_owned());
            screen.visible.push_str(s);
            Ok(())
        }
        fn clear_line(&self) -> io::Result<()> {
            let mut screen = self.0.lock().unwrap();
            screen.visible.clear();
            screen.clears += 1;
            Ok(())
        }
        fn flush(&self) -> io::Result<()> {
            Ok(())
        }
    }

    fn palette() -> Palette {
        Palette {
            theme: Theme::Dark,
            depth: ColorDepth::None,
        }
    }

    #[tokio::test]
    async fn spinner_animates_real_operation_and_clears_on_success_and_error() {
        for result in [Ok(()), Err(crate::Failure::Network)] {
            let term = TestTerm::default();
            let check = term.clone();
            let target = ProgressDrawTarget::term_like(Box::new(term));
            let returned = with_target(target, palette(), async {
                tokio::time::timeout(Duration::from_secs(3), async {
                    loop {
                        let frames = check
                            .0
                            .lock()
                            .unwrap()
                            .writes
                            .iter()
                            .filter(|s| s.contains("Получение треда…"))
                            .count();
                        if frames >= 2 {
                            break;
                        }
                        tokio::time::sleep(Duration::from_millis(10)).await;
                    }
                })
                .await
                .unwrap();
                result
            })
            .await;
            assert_eq!(returned, result);
            let screen = check.0.lock().unwrap();
            assert!(screen.clears >= 2);
            assert!(screen.visible.trim().is_empty());
            let frames: std::collections::HashSet<_> = screen
                .writes
                .iter()
                .filter(|s| s.contains("Получение треда…"))
                .collect();
            assert!(frames.len() >= 2);
            assert!(!screen.writes.join("").contains('%'));
        }
    }

    #[tokio::test]
    async fn cancelled_request_drops_spinner_before_final_diagnostic() {
        let term = TestTerm::default();
        {
            let target = ProgressDrawTarget::term_like(Box::new(term.clone()));
            let request = with_target(target, palette(), std::future::pending::<()>());
            tokio::pin!(request);
            assert!(
                tokio::time::timeout(Duration::from_millis(150), request.as_mut())
                    .await
                    .is_err()
            );
            assert!(
                term.0
                    .lock()
                    .unwrap()
                    .writes
                    .iter()
                    .any(|s| s.contains("Получение треда…"))
            );
        }
        let writes = {
            let screen = term.0.lock().unwrap();
            assert!(screen.visible.trim().is_empty());
            assert!(screen.clears > 0);
            screen.writes.len()
        };
        tokio::time::sleep(Duration::from_millis(150)).await;
        assert_eq!(term.0.lock().unwrap().writes.len(), writes);
    }

    #[tokio::test]
    async fn hidden_streams_have_no_animation_or_control_output() {
        let tty = Terminal {
            attended: true,
            ansi: true,
            width: Some(80),
            no_color: false,
            depth: ColorDepth::None,
        };
        assert!(tty.animation());
        for terminal in [
            Terminal {
                attended: false,
                ..tty
            },
            Terminal { ansi: false, ..tty },
            Terminal { width: None, ..tty },
            Terminal {
                width: Some(10),
                ..tty
            },
        ] {
            assert!(!terminal.animation());
            let result = fetching(terminal, palette(), async { 42 }).await;
            assert_eq!(result, 42);
        }
        let bar = Spinner::start(ProgressDrawTarget::hidden(), palette());
        assert!(bar.0.is_hidden());
    }
}
