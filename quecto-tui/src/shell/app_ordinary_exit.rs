use super::App;

impl App {
    pub(crate) fn request_ordinary_exit(&mut self) {
        self.should_exit = true;
    }

    pub(crate) fn set_ordinary_exit_kill_owned(&mut self, kill_owned: bool) {
        self.ordinary_exit_kill_owned = kill_owned;
    }

    pub(crate) async fn finalize_ordinary_exit(&mut self) {
        self.request_ordinary_exit();
        let _ = self.enqueue_ordinary_exit_snapshot_persists();
        if self.ordinary_exit_kill_owned {
            let watches = self.take_all_child_exit_watches();
            for watch in watches {
                watch.terminate().await;
            }
        }
        self.kitty.cleanup();
        self.terminal.show_cursor();
        self.terminal.exit_raw_mode();
        self.terminal.write_str("\r\n");
    }
}

#[cfg(test)]
#[path = "app_ordinary_exit_tests.rs"]
mod tests;
