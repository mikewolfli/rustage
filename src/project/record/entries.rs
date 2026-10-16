// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! What a project holds, classified: its entries and its tree (task T02.04).
//!
//! A project document says what a project is; this module says what it *has*, in the shape a project
//! tree shows. It is a **view**: it reads the document and the filesystem and produces values, and it
//! has no way to change either. Nothing here is stored, so nothing here can disagree with the
//! document it was built from.
//!
//! Two things are deliberately classified rather than listed flat:
//!
//! * **Entries** are the pages a project can start — the things a target can be pointed at. An entry
//!   carries what it points at, whether that is there, and whether it is the one the target starts
//!   with, because those are the three questions somebody asks about an entry.
//! * **The tree** groups everything by what it is (pages, resources, generated files, stylesheets,
//!   targets, network services) and says, per item, whether the file behind it is present, missing or
//!   refused. A tree that only listed paths would hide exactly the state a person needs to see.

use std::fmt;

use crate::project::record::format::{
    GeneratedTarget, PageEntry, ProjectDocument, ReferenceVerdict, ResourceEntry, TargetProfile,
};

/// One entry: a page a project declares, and what it is for.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Entry {
    written: String,
    title: Option<String>,
    startup: bool,
    document: ReferenceVerdict,
    stylesheet: Option<ReferenceVerdict>,
    generated: Vec<String>,
}

impl Entry {
    /// The page document, as the project writes it.
    pub fn written(&self) -> &str {
        &self.written
    }

    /// The name a person sees, when the document gives one.
    pub fn title(&self) -> Option<&str> {
        self.title.as_deref()
    }

    /// Whether the target starts with this entry.
    pub const fn is_startup(&self) -> bool {
        self.startup
    }

    /// What resolving the page document found.
    pub const fn document(&self) -> &ReferenceVerdict {
        &self.document
    }

    /// What resolving the stylesheet found, when there is one.
    pub const fn stylesheet(&self) -> Option<&ReferenceVerdict> {
        self.stylesheet.as_ref()
    }

    /// The generated files that come from this entry.
    pub fn generated(&self) -> &[String] {
        &self.generated
    }

    /// Whether the page document is there.
    pub fn is_present(&self) -> bool {
        matches!(self.document, ReferenceVerdict::Present)
    }

    /// One line: the state, the path, the title and the roles.
    pub fn render(&self) -> String {
        let mut line = format!(
            "{:<9} {:<32} {}",
            verdict_word(&self.document),
            self.written,
            self.title.as_deref().unwrap_or("<untitled>")
        );
        if self.startup {
            line.push_str("  [startup]");
        }
        if !self.generated.is_empty() {
            line.push_str(&format!("  → {}", self.generated.join(", ")));
        }
        line
    }
}

/// The entries of a project document, in document order.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ProjectEntries {
    entries: Vec<Entry>,
}

impl ProjectEntries {
    /// Reads the entries out of a document.
    pub fn of(document: &ProjectDocument) -> Self {
        let entry = |page: &PageEntry| Entry {
            written: page.document().written().to_owned(),
            title: page.title().map(str::to_owned),
            startup: document
                .startup()
                .is_some_and(|startup| startup.resolves_like(page.document())),
            document: page.document().verdict().clone(),
            stylesheet: page.stylesheet().map(|sheet| sheet.verdict().clone()),
            generated: document
                .generated()
                .iter()
                .filter(|generated| generated.from().resolves_like(page.document()))
                .map(|generated| generated.output().written().to_owned())
                .collect(),
        };
        Self {
            entries: document.pages().iter().map(entry).collect(),
        }
    }

    /// Every entry, in document order.
    pub fn all(&self) -> &[Entry] {
        &self.entries
    }

    /// How many entries there are.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the project has no entries.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The entry at an index.
    pub fn get(&self, index: usize) -> Option<&Entry> {
        self.entries.get(index)
    }

    /// The entry a target starts with, when one is chosen and it is one of these.
    pub fn startup(&self) -> Option<&Entry> {
        self.entries.iter().find(|entry| entry.startup)
    }

    /// The index of the entry a document writes, by the path it writes.
    pub fn index_of(&self, written: &str) -> Option<usize> {
        self.entries
            .iter()
            .position(|entry| entry.written == written)
    }

    /// One line per entry, with what a person has to know about each.
    pub fn render(&self) -> Vec<String> {
        if self.entries.is_empty() {
            return vec!["no entries: this project declares no pages".to_owned()];
        }
        let mut lines: Vec<String> = self.entries.iter().map(Entry::render).collect();
        if self.startup().is_none() {
            lines.push(
                "no startup entry is chosen, so a target has nothing to start yet".to_owned(),
            );
        }
        lines
    }
}

/// What a category of the tree holds.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Category {
    /// The pages, which are also the entries.
    Pages,
    /// The project's own files.
    Resources,
    /// The files this build writes.
    Generated,
    /// The stylesheets the pages name.
    Stylesheets,
    /// The target profiles, which are public configuration.
    Targets,
    /// The network services the project may reach.
    Network,
}

impl Category {
    /// Every category, in the order a tree shows them.
    pub const ALL: [Self; 6] = [
        Self::Pages,
        Self::Resources,
        Self::Generated,
        Self::Stylesheets,
        Self::Targets,
        Self::Network,
    ];

    /// The word a tree uses.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Pages => "pages",
            Self::Resources => "resources",
            Self::Generated => "generated",
            Self::Stylesheets => "stylesheets",
            Self::Targets => "targets",
            Self::Network => "network",
        }
    }
}

impl fmt::Display for Category {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.name())
    }
}

/// One item of a category.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Item {
    /// The first column: a path, a name, or what the item is.
    pub label: String,
    /// The second column: what state it is in.
    pub state: String,
    /// The third column: what it is for.
    pub note: String,
}

/// What a project has, grouped the way a project tree groups it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ProjectTree {
    root: String,
    name: String,
    categories: Vec<(Category, Vec<Item>)>,
}

impl ProjectTree {
    /// Builds the tree a document describes, with what is on disk behind it.
    pub fn of(document: &ProjectDocument, root: Option<&str>) -> Self {
        let entries = ProjectEntries::of(document);
        let mut categories = Vec::new();

        let pages = entries
            .all()
            .iter()
            .map(|entry| Item {
                label: entry.written().to_owned(),
                state: verdict_word(entry.document()).to_owned(),
                note: {
                    let mut note = entry
                        .title()
                        .map(str::to_owned)
                        .unwrap_or_else(|| "<untitled>".to_owned());
                    if entry.is_startup() {
                        note.push_str("  [startup]");
                    }
                    note
                },
            })
            .collect();
        categories.push((Category::Pages, pages));

        categories.push((
            Category::Resources,
            document
                .resources()
                .iter()
                .map(|resource: &ResourceEntry| Item {
                    label: resource.path().written().to_owned(),
                    state: verdict_word(resource.path().verdict()).to_owned(),
                    note: "project file".to_owned(),
                })
                .collect(),
        ));

        categories.push((
            Category::Generated,
            document
                .generated()
                .iter()
                .map(|generated: &GeneratedTarget| Item {
                    label: generated.output().written().to_owned(),
                    state: verdict_word(generated.output().verdict()).to_owned(),
                    note: format!("from {}", generated.from().written()),
                })
                .collect(),
        ));

        let mut stylesheets = Vec::new();
        for page in document.pages() {
            if let Some(sheet) = page.stylesheet() {
                stylesheets.push(Item {
                    label: sheet.written().to_owned(),
                    state: verdict_word(sheet.verdict()).to_owned(),
                    note: format!("styles {}", page.document().written()),
                });
            }
        }
        categories.push((Category::Stylesheets, stylesheets));

        categories.push((
            Category::Targets,
            document
                .targets()
                .iter()
                .map(|target: &TargetProfile| Item {
                    label: target.name().to_owned(),
                    state: "declared".to_owned(),
                    note: {
                        let mut note = target
                            .profile()
                            .map(|profile| format!("profile {profile}"))
                            .unwrap_or_else(|| "no profile".to_owned());
                        if !target.features().is_empty() {
                            note.push_str(&format!(", features {}", target.features().join(" ")));
                        }
                        note
                    },
                })
                .collect(),
        ));

        categories.push((
            Category::Network,
            document
                .services()
                .iter()
                .map(|service| {
                    let policy = service.policy();
                    Item {
                        label: policy.service().to_owned(),
                        state: "declared".to_owned(),
                        note: format!(
                            "{}:{}{}",
                            policy.host(),
                            policy.port(),
                            match policy.credential() {
                                Some(credential) => {
                                    format!(", credential {}", credential.name())
                                }
                                None => String::new(),
                            }
                        ),
                    }
                })
                .collect(),
        ));

        Self {
            root: root.unwrap_or("<unknown>").to_owned(),
            name: document.identity().name().to_owned(),
            categories,
        }
    }

    /// The project's name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The directory the tree is about.
    pub fn root(&self) -> &str {
        &self.root
    }

    /// The categories and what each holds.
    pub fn categories(&self) -> &[(Category, Vec<Item>)] {
        &self.categories
    }

    /// How many items a category holds.
    pub fn len_of(&self, category: Category) -> usize {
        self.categories
            .iter()
            .find(|(known, _)| *known == category)
            .map(|(_, items)| items.len())
            .unwrap_or_default()
    }

    /// One line per item, grouped by category.
    pub fn render(&self) -> Vec<String> {
        let mut lines = vec![format!("{} ({})", self.name, self.root)];
        for (category, items) in &self.categories {
            lines.push(format!("  {} ({})", category.name(), items.len()));
            for item in items {
                lines.push(format!(
                    "    {:<9} {:<32} {}",
                    item.state, item.label, item.note
                ));
            }
        }
        lines
    }
}

/// The word a report uses for a verdict.
fn verdict_word(verdict: &ReferenceVerdict) -> &'static str {
    match verdict {
        ReferenceVerdict::Present => "present",
        ReferenceVerdict::Absent => "missing",
        ReferenceVerdict::Refused { .. } => "refused",
    }
}

#[cfg(test)]
mod tests {
    use super::{Category, ProjectEntries, ProjectTree};
    use crate::host::paths::ProjectRoot;
    use crate::project::record::format::{ProjectDocument, ProjectFile};
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct TempDirectory(PathBuf);

    impl Drop for TempDirectory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    struct Sandbox {
        root: PathBuf,
        _guard: TempDirectory,
    }

    fn sandbox() -> Sandbox {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
        let root =
            std::env::temp_dir().join(format!("rustage-entries-{}-{id}", std::process::id()));
        std::fs::create_dir_all(&root).expect("a temporary directory is creatable");
        Sandbox {
            root: root.clone(),
            _guard: TempDirectory(root),
        }
    }

    impl Sandbox {
        fn root(&self) -> ProjectRoot {
            ProjectRoot::new(&self.root).expect("the sandbox is a directory")
        }

        fn write(&self, relative: &str) {
            let path = self.root.join(relative);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).expect("a directory");
            }
            std::fs::write(&path, "{}").expect("a file");
        }

        fn document(&self, text: &str) -> ProjectDocument {
            ProjectFile::parse(&self.root(), self.root.join("project.json"), text)
                .expect("a readable document")
                .content()
                .document()
                .expect("an interpreted document")
                .clone()
        }
    }

    const DOCUMENT: &str = "{\"format\": 2, \
         \"project\": {\"id\": \"3f2b\", \"name\": \"Demo\"}, \
         \"pages\": [\
           {\"document\": \"pages/main.json\", \"title\": \"Main window\", \"stylesheet\": \"styles/main.css\"}, \
           {\"document\": \"pages/settings.json\"}], \
         \"resources\": [{\"path\": \"resources/icon.svg\"}], \
         \"targets\": [{\"name\": \"desktop\", \"profile\": \"debug\", \"features\": [\"desktop\"]}], \
         \"generated\": [{\"output\": \"src/generated/main.rs\", \"from\": \"pages/main.json\"}], \
         \"startup\": \"pages/settings.json\"}";

    #[test]
    fn the_entries_are_the_pages_and_say_who_starts_and_who_generates() {
        let sandbox = sandbox();
        sandbox.write("pages/main.json");
        sandbox.write("styles/main.css");
        sandbox.write("resources/icon.svg");

        let document = sandbox.document(DOCUMENT);
        let entries = ProjectEntries::of(&document);

        assert_eq!(entries.len(), 2);
        assert!(!entries.is_empty());
        let main = entries.get(0).expect("the first entry");
        assert_eq!(main.written(), "pages/main.json");
        assert_eq!(main.title(), Some("Main window"));
        assert!(!main.is_startup());
        assert!(main.is_present());
        assert_eq!(main.generated(), ["src/generated/main.rs"]);
        assert!(!main.render().contains("[startup]"));
        assert!(main.render().contains("→ src/generated/main.rs"));

        let settings = entries.get(1).expect("the second entry");
        assert_eq!(settings.title(), None);
        assert!(settings.is_startup());
        assert!(!settings.is_present(), "nothing is there yet");
        assert!(settings.render().contains("<untitled>"));
        assert!(settings.render().contains("[startup]"));

        assert_eq!(
            entries.startup().map(|entry| entry.written()),
            Some("pages/settings.json")
        );
        assert_eq!(entries.index_of("pages/main.json"), Some(0));
        assert_eq!(entries.index_of("pages/nowhere.json"), None);
        assert_eq!(entries.get(9), None);
    }

    #[test]
    fn a_project_with_no_entries_says_exactly_that() {
        let sandbox = sandbox();
        let document = sandbox.document(
            "{\"format\": 2, \"project\": {\"id\": \"a\", \"name\": \"Empty\"}, \"pages\": []}",
        );
        let entries = ProjectEntries::of(&document);
        assert!(entries.is_empty());
        assert_eq!(entries.startup(), None);
        assert_eq!(
            entries.render(),
            ["no entries: this project declares no pages".to_owned()]
        );
    }

    #[test]
    fn the_tree_groups_what_a_project_has_and_how_it_is_doing() {
        let sandbox = sandbox();
        sandbox.write("pages/main.json");
        sandbox.write("styles/main.css");
        sandbox.write("resources/icon.svg");

        let document = sandbox.document(DOCUMENT);
        let tree = ProjectTree::of(&document, Some("demo"));

        assert_eq!(tree.name(), "Demo");
        assert_eq!(tree.root(), "demo");
        assert_eq!(tree.len_of(Category::Pages), 2);
        assert_eq!(tree.len_of(Category::Resources), 1);
        assert_eq!(tree.len_of(Category::Generated), 1);
        assert_eq!(tree.len_of(Category::Stylesheets), 1);
        assert_eq!(tree.len_of(Category::Targets), 1);
        assert_eq!(tree.len_of(Category::Network), 0);
        assert_eq!(Category::ALL.len(), 6);
        assert_eq!(Category::Pages.name(), "pages");
        assert_eq!(Category::Pages.to_string(), "pages");
        assert_eq!(tree.categories().len(), 6);

        let rendered = tree.render();
        assert!(rendered[0].contains("Demo (demo)"));
        assert!(rendered.iter().any(|line| line.contains("pages (2)")));
        assert!(
            rendered
                .iter()
                .any(|line| line.contains("missing") && line.contains("pages/settings.json"))
        );
        assert!(
            rendered
                .iter()
                .any(|line| line.contains("present") && line.contains("resources/icon.svg"))
        );
        assert!(
            rendered
                .iter()
                .any(|line| line.contains("from pages/main.json"))
        );
        assert!(
            rendered
                .iter()
                .any(|line| line.contains("styles pages/main.json"))
        );
        assert!(
            rendered
                .iter()
                .any(|line| line.contains("profile debug") && line.contains("features desktop"))
        );
    }

    #[test]
    fn a_reference_that_leaves_the_root_shows_up_as_refused_in_the_tree() {
        let sandbox = sandbox();
        let document = sandbox.document(
            "{\"format\": 2, \"project\": {\"id\": \"a\", \"name\": \"Escaping\"}, \
             \"pages\": [{\"document\": \"../outside.json\"}]}",
        );
        let tree = ProjectTree::of(&document, None);
        assert_eq!(tree.root(), "<unknown>");
        let rendered = tree.render();
        assert!(
            rendered.iter().any(|line| line.contains("refused")),
            "{rendered:?}"
        );
        assert!(rendered.iter().any(|line| line.contains("../outside.json")));
        let entries = ProjectEntries::of(&document);
        assert_eq!(
            entries.all()[0].document(),
            &crate::project::record::format::ReferenceVerdict::Refused {
                reason: "../outside.json climbs above the project root".to_owned(),
            }
        );
    }
}
