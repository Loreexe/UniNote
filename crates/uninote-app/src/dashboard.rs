use crate::icons::{create_icon, IconKind};
use crate::preferences::{DashboardRecentStore, RecentDocumentMeta};
use gtk4::cairo::{Context, Format, ImageSurface};
use gtk4::gio;
use gtk4::glib;
use gtk4::prelude::*;
use gtk4::{
    Align, Box as GtkBox, Button, DropDown, Entry, FileDialog, FlowBox, FlowBoxChild, GestureClick,
    Label, Orientation, Picture, ScrolledWindow, SearchEntry, Window,
};
use std::cell::RefCell;
use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use uninote_core::model::{DocumentFormat, PageBackground};

/// Cartella predefinita per la cache delle miniature Cairo.
pub fn thumbnail_cache_dir() -> PathBuf {
    glib::user_cache_dir().join("uninote").join("thumbnails")
}

/// Genera o restituisce il percorso della miniatura PNG associata al documento.
pub fn get_or_create_thumbnail_path(meta: &RecentDocumentMeta) -> PathBuf {
    if let Some(ref existing) = meta.thumbnail_path {
        let p = PathBuf::from(existing);
        if p.exists() {
            return p;
        }
    }

    let cache_dir = thumbnail_cache_dir();
    let sanitized_title: String = meta
        .title
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '_' })
        .collect();

    let file_hash = {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let mut hasher = DefaultHasher::new();
        meta.file_path.hash(&mut hasher);
        hasher.finish()
    };
    let thumb_filename = format!("{}_{:x}.png", sanitized_title, file_hash);
    let thumb_path = cache_dir.join(thumb_filename);

    if !thumb_path.exists() {
        let _ = render_cairo_thumbnail(&meta.title, meta.format, &thumb_path);
    }
    thumb_path
}

/// Restituisce l'etichetta testuale del badge DMS per il formato documento specificato.
pub fn format_badge_label(format: DocumentFormat) -> &'static str {
    match format {
        DocumentFormat::Notebook => "Quaderno A4",
        DocumentFormat::Whiteboard => "Lavagna Infinita",
    }
}

/// Genera una miniatura visiva 240x340 px in Cairo per una card del quaderno o della lavagna.
pub fn render_cairo_thumbnail(
    _title: &str,
    format: DocumentFormat,
    output_png: &Path,
) -> Result<(), String> {
    let (w, h) = (240, 340);
    let surface = ImageSurface::create(Format::ARgb32, w, h)
        .map_err(|e| format!("Thumbnail surface error: {:?}", e))?;
    let cr = Context::new(&surface).map_err(|e| format!("Thumbnail context error: {:?}", e))?;

    // Sfondo principale Deep Slate (#172126)
    cr.set_source_rgb(0.090, 0.129, 0.149);
    cr.rectangle(0.0, 0.0, w as f64, h as f64);
    cr.fill().map_err(|e| format!("{:?}", e))?;

    match format {
        DocumentFormat::Notebook => {
            // Rigature orizzontali tenui di anteprima foglio
            cr.set_source_rgba(0.427, 0.824, 1.0, 0.06);
            cr.set_line_width(0.8);
            let mut y = 50.0;
            while y < h as f64 - 20.0 {
                cr.move_to(20.0, y);
                cr.line_to(w as f64 - 20.0, y);
                let _ = cr.stroke();
                y += 18.0;
            }

            // Margine verticale sinistro (rosso/corallo DMS tenue)
            cr.set_source_rgba(1.0, 0.706, 0.670, 0.15);
            cr.move_to(45.0, 40.0);
            cr.line_to(45.0, h as f64 - 20.0);
            let _ = cr.stroke();

            // Intestazione quaderno in ciano elettrico DMS
            cr.set_source_rgba(0.427, 0.824, 1.0, 0.25);
            cr.rectangle(20.0, 20.0, w as f64 - 40.0, 15.0);
            cr.fill().map_err(|e| format!("{:?}", e))?;
        }
        DocumentFormat::Whiteboard => {
            // Matrice di punti per la lavagna procedurale
            cr.set_source_rgba(0.475, 0.851, 0.647, 0.12);
            let mut x = 24.0;
            while x < w as f64 - 20.0 {
                let mut y = 24.0;
                while y < h as f64 - 20.0 {
                    cr.arc(x, y, 1.2, 0.0, 2.0 * std::f64::consts::PI);
                    cr.fill().map_err(|e| format!("{:?}", e))?;
                    y += 24.0;
                }
                x += 24.0;
            }

            // Simbolo centrale verde menta DMS
            cr.set_source_rgba(0.475, 0.851, 0.647, 0.25);
            cr.arc(
                w as f64 / 2.0,
                h as f64 / 2.0,
                40.0,
                0.0,
                2.0 * std::f64::consts::PI,
            );
            cr.fill().map_err(|e| format!("{:?}", e))?;
        }
    }

    if let Some(parent) = output_png.parent() {
        if !parent.exists() {
            let _ = fs::create_dir_all(parent);
        }
    }

    let mut file = fs::File::create(output_png).map_err(|e| e.to_string())?;
    surface
        .write_to_png(&mut file)
        .map_err(|e| format!("PNG write error: {:?}", e))?;

    Ok(())
}

/// Dialogo modale per la scelta del formato alla creazione di un nuovo documento.
pub struct CreateDocumentDialog;

impl CreateDocumentDialog {
    pub fn show<F>(parent: Option<&impl IsA<Window>>, on_created: F)
    where
        F: Fn(String, DocumentFormat, PageBackground) + 'static,
    {
        let window = Window::builder()
            .title("Nuovo Documento — UniNote")
            .modal(true)
            .default_width(520)
            .default_height(480)
            .css_classes(["uninote-dialog", "dms-dialog"])
            .build();

        if let Some(p) = parent {
            window.set_transient_for(Some(p.as_ref()));
        }

        let main_box = GtkBox::builder()
            .orientation(Orientation::Vertical)
            .spacing(18)
            .margin_start(28)
            .margin_end(28)
            .margin_top(24)
            .margin_bottom(24)
            .build();

        let lbl_title = Label::builder()
            .label("Crea Nuovo Documento")
            .xalign(0.0)
            .css_classes(["title-1", "brand-title"])
            .build();
        main_box.append(&lbl_title);

        let lbl_subtitle = Label::builder()
            .label("Scegli il formato e le opzioni di layout per il tuo studio")
            .xalign(0.0)
            .css_classes(["dim-label"])
            .build();
        main_box.append(&lbl_subtitle);

        // Nome Documento
        let lbl_name = Label::builder()
            .label("Titolo Documento:")
            .xalign(0.0)
            .css_classes(["heading"])
            .build();
        main_box.append(&lbl_name);

        let entry_name = Entry::builder()
            .text("")
            .placeholder_text("Nuova nota (lascia vuoto per nome automatico)")
            .css_classes(["dms-entry"])
            .build();
        main_box.append(&entry_name);

        // Selezione Formato
        let lbl_format = Label::builder()
            .label("Formato di Studio:")
            .xalign(0.0)
            .css_classes(["heading"])
            .build();
        main_box.append(&lbl_format);

        let format_box = GtkBox::builder()
            .orientation(Orientation::Horizontal)
            .spacing(16)
            .homogeneous(true)
            .build();

        let btn_notebook = Button::builder()
            .label("Quaderno A4\nPagine discrete standard")
            .css_classes(["dms-format-card", "selected"])
            .build();

        let btn_whiteboard = Button::builder()
            .label("Lavagna Infinita\nSpazio libero (-∞, +∞)")
            .css_classes(["dms-format-card"])
            .build();

        format_box.append(&btn_notebook);
        format_box.append(&btn_whiteboard);
        main_box.append(&format_box);

        let selected_format = Rc::new(RefCell::new(DocumentFormat::Notebook));

        let sf_nb = selected_format.clone();
        let b_nb = btn_notebook.clone();
        let b_wb = btn_whiteboard.clone();
        btn_notebook.connect_clicked(move |_| {
            *sf_nb.borrow_mut() = DocumentFormat::Notebook;
            b_nb.add_css_class("selected");
            b_wb.remove_css_class("selected");
        });

        let sf_wb = selected_format.clone();
        let b_nb2 = btn_notebook.clone();
        let b_wb2 = btn_whiteboard.clone();
        btn_whiteboard.connect_clicked(move |_| {
            *sf_wb.borrow_mut() = DocumentFormat::Whiteboard;
            b_wb2.add_css_class("selected");
            b_nb2.remove_css_class("selected");
        });

        // Sfondo Pagina (valido per Quaderno A4)
        let lbl_bg = Label::builder()
            .label("Sfondo Pagina:")
            .xalign(0.0)
            .css_classes(["heading"])
            .build();
        main_box.append(&lbl_bg);

        let bg_options = [
            "Quadretti Matematici (5mm)",
            "Righe Universitarie (8mm)",
            "Bianco Naturale",
        ];
        let dropdown_bg = DropDown::from_strings(&bg_options);
        dropdown_bg.set_selected(0);
        dropdown_bg.add_css_class("dms-dropdown");
        main_box.append(&dropdown_bg);

        // Pulsanti azione
        let actions_box = GtkBox::builder()
            .orientation(Orientation::Horizontal)
            .spacing(12)
            .halign(Align::End)
            .margin_top(16)
            .build();

        let btn_cancel = Button::builder()
            .label("Annulla")
            .css_classes(["dms-action-btn"])
            .build();

        let btn_create = Button::builder()
            .label("Crea Documento")
            .css_classes(["suggested-action", "dms-btn-primary"])
            .build();

        actions_box.append(&btn_cancel);
        actions_box.append(&btn_create);
        main_box.append(&actions_box);

        let win_cancel = window.clone();
        btn_cancel.connect_clicked(move |_| {
            win_cancel.close();
        });

        let win_create = window.clone();
        let callback = Rc::new(on_created);
        btn_create.connect_clicked(move |_| {
            let title = entry_name.text().to_string();
            let final_title = if title.trim().is_empty() {
                let store = DashboardRecentStore::load();
                let (auto_name, _) = store.next_available_note_path();
                auto_name
            } else {
                title.trim().to_string()
            };
            let format = *selected_format.borrow();
            let background = match dropdown_bg.selected() {
                1 => PageBackground::Ruled {
                    line_spacing: 22.68,
                    margin_left: 35.0,
                },
                2 => PageBackground::Blank,
                _ => PageBackground::Grid { grid_size: 14.17 },
            };

            win_create.close();
            callback(final_title, format, background);
        });

        window.set_child(Some(&main_box));
        window.present();
    }
}

pub type OpenDocCallback = Rc<RefCell<Option<Box<dyn Fn(PathBuf)>>>>;
pub type CreateDocCallback =
    Rc<RefCell<Option<Box<dyn Fn(String, DocumentFormat, PageBackground)>>>>;
pub type OpenSettingsCallback = Rc<RefCell<Option<Box<dyn Fn()>>>>;
pub type ImportPdfCallback = Rc<RefCell<Option<Box<dyn Fn(PathBuf)>>>>;

/// Componente Hub Documenti / Dashboard iniziale di UniNote.
pub struct UniNoteDashboard {
    container: ScrolledWindow,
    flow_box: FlowBox,
    search_entry: SearchEntry,
    empty_box: GtkBox,
    store: Rc<RefCell<DashboardRecentStore>>,
    on_open_document: OpenDocCallback,
    on_create_document: CreateDocCallback,
    on_open_settings: OpenSettingsCallback,
    on_import_pdf: ImportPdfCallback,
}

impl Default for UniNoteDashboard {
    fn default() -> Self {
        Self::new()
    }
}

impl UniNoteDashboard {
    /// Inizializza una nuova Dashboard caricando l'archivio dei documenti recenti.
    pub fn new() -> Self {
        Self::with_store(Rc::new(RefCell::new(DashboardRecentStore::load())))
    }

    /// Inizializza la Dashboard a partire da uno store condiviso.
    pub fn with_store(store: Rc<RefCell<DashboardRecentStore>>) -> Self {
        let root_box = GtkBox::builder()
            .orientation(Orientation::Vertical)
            .spacing(24)
            .margin_start(32)
            .margin_end(32)
            .margin_top(24)
            .margin_bottom(32)
            .css_classes(["dms-dashboard"])
            .build();

        // =========================================================================
        // BARRA SUPERIORE (Logo, Titolo, Cerca, Nuovo, Apri File, Impostazioni)
        // =========================================================================
        let top_bar = GtkBox::builder()
            .orientation(Orientation::Horizontal)
            .spacing(16)
            .valign(Align::Center)
            .build();

        // Gruppo Brand Logo + Titolo
        let brand_box = GtkBox::builder()
            .orientation(Orientation::Horizontal)
            .spacing(10)
            .valign(Align::Center)
            .css_classes(["dms-dashboard-brand"])
            .build();

        let logo_icon = create_icon(IconKind::AppLogo, 36, None);
        brand_box.append(&logo_icon);

        let brand_labels = GtkBox::builder()
            .orientation(Orientation::Vertical)
            .valign(Align::Center)
            .build();

        let lbl_app_title = Label::builder()
            .label("UniNote")
            .xalign(0.0)
            .css_classes(["title-2", "brand-title"])
            .build();
        brand_labels.append(&lbl_app_title);

        let lbl_hub_subtitle = Label::builder()
            .label("Hub Documenti Accademico")
            .xalign(0.0)
            .css_classes(["dim-label"])
            .build();
        brand_labels.append(&lbl_hub_subtitle);

        brand_box.append(&brand_labels);
        top_bar.append(&brand_box);

        // Separatore flessibile sinistro
        let spacer_left = GtkBox::builder().hexpand(true).build();
        top_bar.append(&spacer_left);

        // Campo di Ricerca Istantanea
        let search_entry = SearchEntry::builder()
            .placeholder_text("Cerca quaderni e appunti...")
            .width_request(340)
            .css_classes(["dms-search-entry"])
            .build();
        top_bar.append(&search_entry);

        // Separatore flessibile destro
        let spacer_right = GtkBox::builder().hexpand(true).build();
        top_bar.append(&spacer_right);

        // Pulsante Primario "+ Nuovo Documento"
        let btn_new_doc = Button::builder()
            .label("+ Nuovo Documento")
            .css_classes(["suggested-action", "dms-btn-primary"])
            .valign(Align::Center)
            .build();
        top_bar.append(&btn_new_doc);

        // Pulsante "Nuovo da PDF..."
        let btn_import_pdf = Button::builder()
            .label("Nuovo da PDF...")
            .tooltip_text("Importa dispense o slide accademiche creando un quaderno")
            .css_classes(["dms-action-btn"])
            .valign(Align::Center)
            .build();
        top_bar.append(&btn_import_pdf);

        // Pulsante Secondario "Apri File..."
        let btn_open_file = Button::builder()
            .label("Apri File...")
            .css_classes(["dms-action-btn"])
            .valign(Align::Center)
            .build();
        top_bar.append(&btn_open_file);

        // Pulsante Impostazioni
        let btn_settings = Button::builder()
            .child(&create_icon(IconKind::Settings, 18, None))
            .tooltip_text("Impostazioni UniNote")
            .css_classes(["dms-action-btn"])
            .valign(Align::Center)
            .build();
        top_bar.append(&btn_settings);

        root_box.append(&top_bar);

        // =========================================================================
        // SEZIONE GRIGLIA CARD RECENTI
        // =========================================================================
        let header_box = GtkBox::builder()
            .orientation(Orientation::Horizontal)
            .spacing(12)
            .margin_top(8)
            .build();

        let lbl_recent_header = Label::builder()
            .label("Documenti Recenti")
            .xalign(0.0)
            .css_classes(["title-3", "heading"])
            .build();
        header_box.append(&lbl_recent_header);

        root_box.append(&header_box);

        let flow_box = FlowBox::builder()
            .valign(Align::Start)
            .halign(Align::Start)
            .column_spacing(24)
            .row_spacing(24)
            .min_children_per_line(1)
            .max_children_per_line(6)
            .selection_mode(gtk4::SelectionMode::None)
            .homogeneous(false)
            .css_classes(["dms-dashboard-grid"])
            .build();
        root_box.append(&flow_box);

        // Box Stato Vuoto
        let empty_box = GtkBox::builder()
            .orientation(Orientation::Vertical)
            .spacing(12)
            .halign(Align::Center)
            .valign(Align::Center)
            .vexpand(true)
            .margin_top(60)
            .margin_bottom(60)
            .build();

        let empty_title = Label::builder()
            .label("Nessun documento trovato")
            .css_classes(["title-2"])
            .build();
        let empty_desc = Label::builder()
            .label("Crea un nuovo quaderno A4 o una lavagna infinita per iniziare a scrivere.")
            .css_classes(["dim-label"])
            .build();

        empty_box.append(&empty_title);
        empty_box.append(&empty_desc);
        empty_box.set_visible(false);
        root_box.append(&empty_box);

        let scrolled = ScrolledWindow::builder()
            .child(&root_box)
            .hscrollbar_policy(gtk4::PolicyType::Never)
            .vscrollbar_policy(gtk4::PolicyType::Automatic)
            .hexpand(true)
            .vexpand(true)
            .build();

        let on_open_document = Rc::new(RefCell::new(None));
        let on_create_document = Rc::new(RefCell::new(None));
        let on_open_settings = Rc::new(RefCell::new(None));
        let on_import_pdf = Rc::new(RefCell::new(None));

        let dashboard = Self {
            container: scrolled,
            flow_box,
            search_entry,
            empty_box,
            store,
            on_open_document,
            on_create_document,
            on_open_settings,
            on_import_pdf,
        };

        dashboard.wire_signals(btn_new_doc, btn_import_pdf, btn_open_file, btn_settings);
        dashboard.refresh();

        dashboard
    }

    /// Collega i segnali di ricerca, apertura file, importazione PDF e creazione documento.
    fn wire_signals(
        &self,
        btn_new: Button,
        btn_import: Button,
        btn_open: Button,
        btn_settings: Button,
    ) {
        let store_ref = self.store.clone();
        let flow_ref = self.flow_box.clone();
        let empty_ref = self.empty_box.clone();
        let open_cb = self.on_open_document.clone();

        self.search_entry.connect_search_changed(move |entry| {
            let query = entry.text().to_string();
            Self::populate_flowbox(&flow_ref, &empty_ref, &store_ref, &query, &open_cb);
        });

        // Pulsante + Nuovo Documento
        let create_cb = self.on_create_document.clone();
        btn_new.connect_clicked(move |btn| {
            let parent_win = btn.root().and_then(|r| r.downcast::<Window>().ok());
            let c_cb = create_cb.clone();
            CreateDocumentDialog::show(parent_win.as_ref(), move |title, format, bg| {
                if let Some(ref cb) = *c_cb.borrow() {
                    cb(title, format, bg);
                }
            });
        });

        // Pulsante Nuovo da PDF...
        let import_cb = self.on_import_pdf.clone();
        let store_for_import = self.store.clone();
        let flow_for_import = self.flow_box.clone();
        let empty_for_import = self.empty_box.clone();
        let search_for_import = self.search_entry.clone();
        let open_for_import = self.on_open_document.clone();

        btn_import.connect_clicked(move |btn| {
            let parent_win = btn.root().and_then(|r| r.downcast::<Window>().ok());
            let dialog = FileDialog::builder()
                .title("Seleziona File PDF da Importare")
                .modal(true)
                .build();

            let filter = gtk4::FileFilter::new();
            filter.set_name(Some("Documenti PDF (*.pdf)"));
            filter.add_pattern("*.pdf");
            filter.add_mime_type("application/pdf");
            let filters = gio::ListStore::new::<gtk4::FileFilter>();
            filters.append(&filter);
            dialog.set_filters(Some(&filters));

            let imp_cb = import_cb.clone();
            let st = store_for_import.clone();
            let fl = flow_for_import.clone();
            let em = empty_for_import.clone();
            let se = search_for_import.clone();
            let od = open_for_import.clone();

            dialog.open(parent_win.as_ref(), gio::Cancellable::NONE, move |res| {
                if let Ok(file) = res {
                    if let Some(path) = file.path() {
                        let path_clone = path.clone();
                        let title = path
                            .file_stem()
                            .and_then(|s| s.to_str())
                            .unwrap_or("Documento PDF")
                            .to_string();

                        let meta = RecentDocumentMeta {
                            file_path: path.to_string_lossy().to_string(),
                            title,
                            format: DocumentFormat::Notebook,
                            last_modified_timestamp: std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .map(|d| d.as_secs())
                                .unwrap_or(0),
                            page_count: 1,
                            thumbnail_path: None,
                        };

                        st.borrow_mut().add_recent(meta);
                        let _ = st.borrow().save();
                        Self::populate_flowbox(&fl, &em, &st, &se.text(), &od);

                        if let Some(ref cb) = *imp_cb.borrow() {
                            cb(path_clone);
                        }
                    }
                }
            });
        });

        // Pulsante Apri File...
        let open_doc_cb = self.on_open_document.clone();
        let store_for_open = self.store.clone();
        let flow_for_open = self.flow_box.clone();
        let empty_for_open = self.empty_box.clone();
        let search_for_open = self.search_entry.clone();
        btn_open.connect_clicked(move |btn| {
            let parent_win = btn.root().and_then(|r| r.downcast::<Window>().ok());
            let dialog = FileDialog::builder()
                .title("Apri Documento UniNote o PDF")
                .modal(true)
                .build();

            let filter = gtk4::FileFilter::new();
            filter.set_name(Some("Documenti UniNote e PDF (*.uninote, *.pdf)"));
            filter.add_pattern("*.uninote");
            filter.add_pattern("*.pdf");
            filter.add_mime_type("application/pdf");
            let filters = gio::ListStore::new::<gtk4::FileFilter>();
            filters.append(&filter);
            dialog.set_filters(Some(&filters));

            let od_cb = open_doc_cb.clone();
            let st = store_for_open.clone();
            let fl = flow_for_open.clone();
            let em = empty_for_open.clone();
            let se = search_for_open.clone();
            dialog.open(parent_win.as_ref(), gio::Cancellable::NONE, move |res| {
                if let Ok(file) = res {
                    if let Some(path) = file.path() {
                        let path_clone = path.clone();
                        // Registra nei recenti
                        let title = path
                            .file_stem()
                            .and_then(|s| s.to_str())
                            .unwrap_or("Documento")
                            .to_string();

                        let meta = RecentDocumentMeta {
                            file_path: path.to_string_lossy().to_string(),
                            title,
                            format: DocumentFormat::Notebook,
                            last_modified_timestamp: std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .map(|d| d.as_secs())
                                .unwrap_or(0),
                            page_count: 1,
                            thumbnail_path: None,
                        };

                        st.borrow_mut().add_recent(meta);
                        let _ = st.borrow().save();
                        Self::populate_flowbox(&fl, &em, &st, &se.text(), &od_cb);

                        if let Some(ref cb) = *od_cb.borrow() {
                            cb(path_clone);
                        }
                    }
                }
            });
        });

        // Pulsante Impostazioni
        let settings_cb = self.on_open_settings.clone();
        btn_settings.connect_clicked(move |_| {
            if let Some(ref cb) = *settings_cb.borrow() {
                cb();
            }
        });
    }

    /// Ricostruisce le card dei quaderni nella `GtkFlowBox` filtrando per la query corrente.
    pub fn refresh(&self) {
        self.store.borrow_mut().sanitize_existing_files();
        let query = self.search_entry.text().to_string();
        Self::populate_flowbox(
            &self.flow_box,
            &self.empty_box,
            &self.store,
            &query,
            &self.on_open_document,
        );
    }

    fn populate_flowbox(
        flow_box: &FlowBox,
        empty_box: &GtkBox,
        store: &Rc<RefCell<DashboardRecentStore>>,
        query: &str,
        open_cb: &OpenDocCallback,
    ) {
        // Rimuove tutti i figli precedenti
        while let Some(child) = flow_box.first_child() {
            flow_box.remove(&child);
        }

        let filtered = store.borrow().filter_by_query(query);

        if filtered.is_empty() {
            empty_box.set_visible(true);
            flow_box.set_visible(false);
            return;
        }

        empty_box.set_visible(false);
        flow_box.set_visible(true);

        for meta in filtered {
            let card = Self::create_card(&meta, open_cb);
            flow_box.append(&card);
        }
    }

    /// Restituisce l'etichetta testuale del badge DMS per il formato documento specificato.
    pub fn format_badge_label(format: DocumentFormat) -> &'static str {
        match format {
            DocumentFormat::Notebook => "Quaderno A4",
            DocumentFormat::Whiteboard => "Lavagna Infinita",
        }
    }

    /// Crea il widget grafico di una singola card quaderno conforme a DMS.
    pub fn create_card(meta: &RecentDocumentMeta, open_cb: &OpenDocCallback) -> FlowBoxChild {
        let child = FlowBoxChild::new();
        child.add_css_class("dms-flowbox-child");

        let card_box = GtkBox::builder()
            .orientation(Orientation::Vertical)
            .spacing(10)
            .width_request(240)
            .css_classes(["dms-dashboard-card"])
            .can_target(true)
            .build();

        // 1. Miniatura 240x340 px
        let thumb_path = get_or_create_thumbnail_path(meta);
        let picture = Picture::for_filename(&thumb_path);
        picture.set_size_request(240, 340);
        picture.set_content_fit(gtk4::ContentFit::Cover);
        picture.add_css_class("dms-card-thumbnail");
        card_box.append(&picture);

        // 2. Dettagli testuali
        let info_box = GtkBox::builder()
            .orientation(Orientation::Vertical)
            .spacing(4)
            .margin_start(10)
            .margin_end(10)
            .margin_bottom(10)
            .build();

        let lbl_title = Label::builder()
            .label(&meta.title)
            .xalign(0.0)
            .ellipsize(gtk4::pango::EllipsizeMode::End)
            .css_classes(["dms-card-title", "heading"])
            .build();
        info_box.append(&lbl_title);

        let meta_row = GtkBox::builder()
            .orientation(Orientation::Horizontal)
            .spacing(8)
            .valign(Align::Center)
            .build();

        let badge_text = Self::format_badge_label(meta.format);

        let badge_class = match meta.format {
            DocumentFormat::Notebook => "badge-notebook",
            DocumentFormat::Whiteboard => "badge-whiteboard",
        };

        let lbl_badge = Label::builder()
            .label(badge_text)
            .css_classes(["dms-format-badge", badge_class])
            .build();
        meta_row.append(&lbl_badge);

        let pages_text = match meta.format {
            DocumentFormat::Notebook => format!("{} pag.", meta.page_count),
            DocumentFormat::Whiteboard => "Infinita".to_string(),
        };

        let lbl_pages = Label::builder()
            .label(&pages_text)
            .css_classes(["dim-label"])
            .build();
        meta_row.append(&lbl_pages);

        info_box.append(&meta_row);
        card_box.append(&info_box);

        // Interattività al click per aprire il documento
        let gesture = GestureClick::new();
        let target_path = PathBuf::from(&meta.file_path);
        let cb_clone = open_cb.clone();
        gesture.connect_pressed(move |_, _, _, _| {
            if let Some(ref cb) = *cb_clone.borrow() {
                cb(target_path.clone());
            }
        });
        card_box.add_controller(gesture);

        child.set_child(Some(&card_box));
        child
    }

    /// Restituisce il widget GTK radice della Dashboard per l'inclusione nella gerarchia Adwaita.
    pub fn widget(&self) -> &gtk4::Widget {
        self.container.upcast_ref()
    }

    /// Connette il callback invocato quando l'utente seleziona un documento da aprire.
    pub fn connect_open_document<F>(&self, callback: F)
    where
        F: Fn(PathBuf) + 'static,
    {
        *self.on_open_document.borrow_mut() = Some(Box::new(callback));
    }

    /// Connette il callback invocato quando l'utente crea un nuovo documento.
    pub fn connect_create_document<F>(&self, callback: F)
    where
        F: Fn(String, DocumentFormat, PageBackground) + 'static,
    {
        *self.on_create_document.borrow_mut() = Some(Box::new(callback));
    }

    /// Connette il callback invocato quando l'utente clicca sul pulsante impostazioni.
    pub fn connect_open_settings<F>(&self, callback: F)
    where
        F: Fn() + 'static,
    {
        *self.on_open_settings.borrow_mut() = Some(Box::new(callback));
    }

    /// Connette il callback invocato quando l'utente importa un file PDF.
    pub fn connect_import_pdf<F>(&self, callback: F)
    where
        F: Fn(PathBuf) + 'static,
    {
        *self.on_import_pdf.borrow_mut() = Some(Box::new(callback));
    }

    /// Restituisce il riferimento condiviso allo store dei documenti recenti.
    pub fn store(&self) -> Rc<RefCell<DashboardRecentStore>> {
        self.store.clone()
    }
}
