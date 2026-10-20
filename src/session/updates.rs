//! Checking for and installing a newer release (see [`crate::update`]).

use super::{Background, Changes, Session};
use crate::update::{self, Installed};
use crate::{tr, trf};

impl Session {
    pub(super) fn update_event(&mut self, event: Background, changes: &mut Changes) {
        match event {
            Background::UpdateChecked { manual, result } => {
                self.checking_update = false;
                match result {
                    Ok(Some(found)) => {
                        self.set_info(trf!(
                            self.lang,
                            "Yarek Music {} is available — Settings → Update (or `yarek update`)",
                            "Доступна Yarek Music {} — Налаштування → Оновити (або `yarek update`)",
                            found.version
                        ));
                        self.update = Some(found);
                    }
                    Ok(None) => {
                        self.update = None;
                        if manual {
                            self.set_info(trf!(
                                self.lang,
                                "Yarek Music {} is the latest version",
                                "Yarek Music {} — найсвіжіша версія",
                                env!("CARGO_PKG_VERSION")
                            ));
                        }
                    }
                    Err(err) if manual => self.set_error(format!("{err:#}")),
                    Err(err) => tracing::info!("update check: {err:#}"),
                }
                changes.update = true;
            }
            Background::UpdateInstalled(result) => {
                self.updating = false;
                match result {
                    Ok(done) => {
                        match &done {
                            Installed::Replaced => self.set_info(tr!(
                                self.lang,
                                "Updated — restarting…",
                                "Оновлено — перезапускаюсь…"
                            )),
                            Installed::InstallerStarted { .. } => self.set_info(tr!(
                                self.lang,
                                "The installer is open — follow it to finish",
                                "Інсталятор відкрито — заверши встановлення в ньому"
                            )),
                            Installed::Manual(how) => self.set_info(how.clone()),
                        }
                        self.update_done = Some(done);
                    }
                    Err(err) => self.set_error(trf!(
                        self.lang,
                        "Update failed: {err:#}",
                        "Оновлення не вдалось: {err:#}"
                    )),
                }
                changes.update = true;
            }
            _ => unreachable!("not an update event"),
        }
    }

    /// Asks GitHub for a newer release; `manual` reports "up to date" too.
    pub fn check_for_update(&mut self, manual: bool) {
        if self.checking_update {
            return;
        }
        self.checking_update = true;
        let (http, tx) = (self.deps.http.clone(), self.tx.clone());
        tokio::spawn(async move {
            let result = update::check(&http).await;
            let _ = tx.send(Background::UpdateChecked { manual, result });
        });
    }

    /// Downloads and installs the update found by the last check.
    pub fn install_update(&mut self) {
        let Some(found) = self.update.clone() else {
            return;
        };
        if self.updating {
            return;
        }
        self.updating = true;
        self.set_info(trf!(
            self.lang,
            "Downloading Yarek Music {}…",
            "Завантажую Yarek Music {}…",
            found.version
        ));
        let (http, tx) = (self.deps.http.clone(), self.tx.clone());
        tokio::spawn(async move {
            let result = update::install(&http, &found).await;
            let _ = tx.send(Background::UpdateInstalled(result));
        });
    }
}
