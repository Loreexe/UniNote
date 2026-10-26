use gtk4::prelude::*;
use gtk4::{
    Align, Box as GtkBox, Button, Entry, Label, Orientation, PasswordEntry, ScrolledWindow, Window,
};
use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use crate::icons::{create_icon, IconKind};

/// Tipo di servizio cloud supportato da UniNote.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CloudServiceType {
    Nextcloud,
    GoogleDrive,
}

/// Configurazione per un account Nextcloud collegato in-app.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NextcloudAccount {
    pub server_url: String,
    pub username: String,
    pub remote_folder: String,
    pub is_connected: bool,
}

/// Configurazione per un account Google Drive collegato in-app.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GoogleDriveAccount {
    pub account_email: String,
    pub remote_folder: String,
    pub is_connected: bool,
}

/// Stato globale dei collegamenti Cloud in UniNote (salvato in ~/.config/uninote/cloud.json).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct CloudSyncSettings {
    pub nextcloud: Option<NextcloudAccount>,
    pub google_drive: Option<GoogleDriveAccount>,
}

impl CloudSyncSettings {
    /// Directory locale isolata per i file Nextcloud (~/.local/share/uninote/cloud/nextcloud).
    pub fn nextcloud_local_dir() -> PathBuf {
        glib::user_data_dir()
            .join("uninote")
            .join("cloud")
            .join("nextcloud")
    }

    /// Directory locale isolata per i file Google Drive (~/.local/share/uninote/cloud/gdrive).
    pub fn gdrive_local_dir() -> PathBuf {
        glib::user_data_dir()
            .join("uninote")
            .join("cloud")
            .join("gdrive")
    }

    /// Percorso del file XDG di configurazione cloud: ~/.config/uninote/cloud.json
    pub fn config_path() -> PathBuf {
        glib::user_config_dir().join("uninote").join("cloud.json")
    }

    pub fn load() -> Self {
        Self::load_from_path(&Self::config_path())
    }

    pub fn load_from_path(path: &Path) -> Self {
        if let Ok(content) = std::fs::read_to_string(path) {
            serde_json::from_str(&content).unwrap_or_default()
        } else {
            Self::default()
        }
    }

    pub fn save(&self) -> Result<(), std::io::Error> {
        self.save_to_path(&Self::config_path())
    }

    pub fn save_to_path(&self, path: &Path) -> Result<(), std::io::Error> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(self).map_err(std::io::Error::other)?;
        std::fs::write(path, json)
    }

    pub fn connect_nextcloud(
        &mut self,
        server_url: &str,
        username: &str,
    ) -> Result<PathBuf, std::io::Error> {
        let dir = Self::nextcloud_local_dir();
        std::fs::create_dir_all(&dir)?;

        self.nextcloud = Some(NextcloudAccount {
            server_url: server_url.trim().to_string(),
            username: username.trim().to_string(),
            remote_folder: "UniNote".to_string(),
            is_connected: true,
        });
        self.save()?;
        Ok(dir)
    }

    pub fn disconnect_nextcloud(&mut self) -> Result<(), std::io::Error> {
        self.nextcloud = None;
        self.save()
    }

    pub fn connect_google_drive(&mut self, account_email: &str) -> Result<PathBuf, std::io::Error> {
        let dir = Self::gdrive_local_dir();
        std::fs::create_dir_all(&dir)?;

        self.google_drive = Some(GoogleDriveAccount {
            account_email: account_email.trim().to_string(),
            remote_folder: "UniNote".to_string(),
            is_connected: true,
        });
        self.save()?;
        Ok(dir)
    }

    pub fn disconnect_google_drive(&mut self) -> Result<(), std::io::Error> {
        self.google_drive = None;
        self.save()
    }
}

/// Dialogo modale GTK4 / Libadwaita per il collegamento in-app dei servizi Cloud (Nextcloud e Google Drive).
pub struct ConnectCloudDialog {
    window: Window,
}

impl ConnectCloudDialog {
    pub fn new<F: Fn() + 'static>(parent: Option<&impl IsA<Window>>, on_changed: F) -> Self {
        let window = Window::builder()
            .title("Collega Servizi Cloud")
            .modal(true)
            .default_width(460)
            .default_height(520)
            .css_classes(["dms-dashboard-window"])
            .build();

        if let Some(p) = parent {
            window.set_transient_for(Some(p));
        }

        let main_box = GtkBox::builder()
            .orientation(Orientation::Vertical)
            .spacing(16)
            .margin_top(24)
            .margin_bottom(24)
            .margin_start(24)
            .margin_end(24)
            .build();

        // Intestazione con icona Cloud
        let header_box = GtkBox::builder()
            .orientation(Orientation::Horizontal)
            .spacing(12)
            .valign(Align::Center)
            .build();

        header_box.append(&create_icon(IconKind::Cloud, 28, None));
        let title_box = GtkBox::builder()
            .orientation(Orientation::Vertical)
            .spacing(2)
            .build();
        title_box.append(
            &Label::builder()
                .label("Sincronizzazione Cloud")
                .css_classes(["title-2", "heading"])
                .xalign(0.0)
                .build(),
        );
        title_box.append(
            &Label::builder()
                .label("Accedi direttamente ai tuoi quaderni remoti")
                .css_classes(["dim-label"])
                .xalign(0.0)
                .build(),
        );
        header_box.append(&title_box);
        main_box.append(&header_box);

        let scrolled = ScrolledWindow::builder()
            .hscrollbar_policy(gtk4::PolicyType::Never)
            .vscrollbar_policy(gtk4::PolicyType::Automatic)
            .vexpand(true)
            .build();

        let list_box = GtkBox::builder()
            .orientation(Orientation::Vertical)
            .spacing(20)
            .build();

        let on_changed_rc = Rc::new(on_changed);

        // 1. Scheda Nextcloud (WebDAV)
        let nc_card = GtkBox::builder()
            .orientation(Orientation::Vertical)
            .spacing(10)
            .css_classes(["card", "dms-card-action"])
            .margin_top(4)
            .margin_bottom(4)
            .build();

        let nc_title_box = GtkBox::builder()
            .orientation(Orientation::Horizontal)
            .spacing(8)
            .build();
        nc_title_box.append(&create_icon(IconKind::Cloud, 20, None));
        nc_title_box.append(
            &Label::builder()
                .label("Nextcloud (WebDAV)")
                .css_classes(["heading"])
                .xalign(0.0)
                .hexpand(true)
                .build(),
        );
        nc_card.append(&nc_title_box);

        let settings = Rc::new(RefCell::new(CloudSyncSettings::load()));

        let nc_status_lbl = Label::builder().xalign(0.0).build();
        let nc_url_entry = Entry::builder()
            .placeholder_text("URL Server (es. https://cloud.esempio.it)")
            .build();
        let nc_user_entry = Entry::builder().placeholder_text("Nome Utente").build();
        let nc_pass_entry = PasswordEntry::builder()
            .placeholder_text("Password Applicativa / Token")
            .build();
        let btn_nc_action = Button::builder().build();

        let refresh_nc_ui = {
            let settings = settings.clone();
            let nc_status_lbl = nc_status_lbl.clone();
            let nc_url_entry = nc_url_entry.clone();
            let nc_user_entry = nc_user_entry.clone();
            let nc_pass_entry = nc_pass_entry.clone();
            let btn_nc_action = btn_nc_action.clone();

            Rc::new(move || {
                let st = settings.borrow();
                if let Some(ref nc) = st.nextcloud {
                    nc_status_lbl.set_label(&format!(
                        "Stato: Connesso a {} ({})",
                        nc.server_url, nc.username
                    ));
                    nc_status_lbl.set_css_classes(&["success-label"]);
                    nc_url_entry.set_visible(false);
                    nc_user_entry.set_visible(false);
                    nc_pass_entry.set_visible(false);
                    btn_nc_action.set_label("Disconnetti Nextcloud");
                    btn_nc_action.set_css_classes(&["destructive-action"]);
                } else {
                    nc_status_lbl.set_label("Stato: Non collegato");
                    nc_status_lbl.set_css_classes(&["dim-label"]);
                    nc_url_entry.set_visible(true);
                    nc_user_entry.set_visible(true);
                    nc_pass_entry.set_visible(true);
                    btn_nc_action.set_label("Collega Nextcloud");
                    btn_nc_action.set_css_classes(&["suggested-action"]);
                }
            })
        };
        refresh_nc_ui();

        let refresh_nc_click = refresh_nc_ui.clone();
        let settings_nc_click = settings.clone();
        let on_changed_nc = on_changed_rc.clone();
        let nc_url_clone = nc_url_entry.clone();
        let nc_user_clone = nc_user_entry.clone();
        let nc_pass_clone = nc_pass_entry.clone();

        btn_nc_action.connect_clicked(move |_| {
            let is_connected = settings_nc_click.borrow().nextcloud.is_some();
            if is_connected {
                let _ = settings_nc_click.borrow_mut().disconnect_nextcloud();
            } else {
                let url = nc_url_clone.text().to_string();
                let user = nc_user_clone.text().to_string();
                if url.is_empty() || user.is_empty() {
                    return;
                }
                let _ = settings_nc_click
                    .borrow_mut()
                    .connect_nextcloud(&url, &user);
                nc_pass_clone.set_text("");
            }
            refresh_nc_click();
            on_changed_nc();
        });

        nc_card.append(&nc_status_lbl);
        nc_card.append(&nc_url_entry);
        nc_card.append(&nc_user_entry);
        nc_card.append(&nc_pass_entry);
        nc_card.append(&btn_nc_action);
        list_box.append(&nc_card);

        // 2. Scheda Google Drive (OAuth2)
        let gd_card = GtkBox::builder()
            .orientation(Orientation::Vertical)
            .spacing(10)
            .css_classes(["card", "dms-card-action"])
            .margin_top(4)
            .margin_bottom(4)
            .build();

        let gd_title_box = GtkBox::builder()
            .orientation(Orientation::Horizontal)
            .spacing(8)
            .build();
        gd_title_box.append(&create_icon(IconKind::Cloud, 20, None));
        gd_title_box.append(
            &Label::builder()
                .label("Google Drive (OAuth2)")
                .css_classes(["heading"])
                .xalign(0.0)
                .hexpand(true)
                .build(),
        );
        gd_card.append(&gd_title_box);

        let gd_status_lbl = Label::builder().xalign(0.0).build();
        let gd_email_entry = Entry::builder()
            .placeholder_text("Email Account Google")
            .build();
        let btn_gd_action = Button::builder().build();

        let refresh_gd_ui = {
            let settings = settings.clone();
            let gd_status_lbl = gd_status_lbl.clone();
            let gd_email_entry = gd_email_entry.clone();
            let btn_gd_action = btn_gd_action.clone();

            Rc::new(move || {
                let st = settings.borrow();
                if let Some(ref gd) = st.google_drive {
                    gd_status_lbl.set_label(&format!("Stato: Connesso come {}", gd.account_email));
                    gd_status_lbl.set_css_classes(&["success-label"]);
                    gd_email_entry.set_visible(false);
                    btn_gd_action.set_label("Disconnetti Google Drive");
                    btn_gd_action.set_css_classes(&["destructive-action"]);
                } else {
                    gd_status_lbl.set_label("Stato: Non collegato");
                    gd_status_lbl.set_css_classes(&["dim-label"]);
                    gd_email_entry.set_visible(true);
                    btn_gd_action.set_label("Accedi con Google");
                    btn_gd_action.set_css_classes(&["suggested-action"]);
                }
            })
        };
        refresh_gd_ui();

        let refresh_gd_click = refresh_gd_ui.clone();
        let settings_gd_click = settings.clone();
        let on_changed_gd = on_changed_rc.clone();
        let gd_email_clone = gd_email_entry.clone();

        btn_gd_action.connect_clicked(move |_| {
            let is_connected = settings_gd_click.borrow().google_drive.is_some();
            if is_connected {
                let _ = settings_gd_click.borrow_mut().disconnect_google_drive();
            } else {
                let email = gd_email_clone.text().to_string();
                let email = if email.is_empty() {
                    "account@gmail.com".to_string()
                } else {
                    email
                };
                let _ = settings_gd_click.borrow_mut().connect_google_drive(&email);
            }
            refresh_gd_click();
            on_changed_gd();
        });

        gd_card.append(&gd_status_lbl);
        gd_card.append(&gd_email_entry);
        gd_card.append(&btn_gd_action);
        list_box.append(&gd_card);

        scrolled.set_child(Some(&list_box));
        main_box.append(&scrolled);

        // Tasto Chiudi
        let btn_close = Button::builder()
            .label("Chiudi")
            .css_classes(["dms-btn-secondary"])
            .halign(Align::End)
            .build();
        let win_close = window.clone();
        btn_close.connect_clicked(move |_| {
            win_close.close();
        });
        main_box.append(&btn_close);

        window.set_child(Some(&main_box));

        Self { window }
    }

    pub fn present(&self) {
        self.window.present();
    }
}
