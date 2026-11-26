//! Strings built on the Rust side (lists, summaries, messages). The static
//! texts of the window are translated by Slint (`lang/`).

use bento_core::i18n::Lang;

macro_rules! texts {
    ($($id:ident => $fr:expr, $en:expr;)*) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub enum T { $($id),* }

        impl T {
            pub fn get(self, lang: Lang) -> &'static str {
                match (self, lang) {
                    $((T::$id, Lang::Fr) => $fr, (T::$id, Lang::En) => $en,)*
                }
            }

            /// Replaces each `{}` in order with the arguments.
            pub fn fmt(self, lang: Lang, args: &[&str]) -> String {
                let mut out = String::new();
                let mut rest = self.get(lang);
                for a in args {
                    match rest.split_once("{}") {
                        Some((head, tail)) => {
                            out.push_str(head);
                            out.push_str(a);
                            rest = tail;
                        }
                        None => break,
                    }
                }
                out.push_str(rest);
                out
            }
        }
    };
}

texts! {
    WindowTitle => "Bento Desk - Paramètres", "Bento Desk - Settings";
    DefaultStyle => "Style par défaut (toutes les fences)", "Default style (all fences)";
    PageIcons => "Page {} · {} icônes", "Page {} · {} icons";
    PageTabs => "Page {} · {} icônes · {} onglets", "Page {} · {} icons · {} tabs";
    Portal => "Page {} · portail : {}", "Page {} · portal: {}";
    Untitled => "(sans titre)", "(untitled)";
    NoFence => "(aucune fence)", "(no fence)";
    NewRule => "Nouvelle règle", "New rule";
    NoCondition => "aucune condition", "no condition";
    And => " et ", " and ";
    Or => " ou ", " or ";
    CondName => "nom {} « {} »", "name {} \"{}\"";
    CondExt => "extension {}", "extension {}";
    CondKind => "type {}", "type {}";
    CondTarget => "cible {} « {} »", "target {} \"{}\"";
    CondDate => "{} il y a {} {} jours", "{} {} {} days ago";
    OpContains => "contient", "contains";
    OpStarts => "commence par", "starts with";
    OpEquals => "est", "is";
    Modified => "modifié", "modified";
    Created => "créé", "created";
    LessThan => "moins de", "less than";
    MoreThan => "plus de", "more than";
    KindShortcut => "raccourci", "shortcut";
    KindFolder => "dossier", "folder";
    KindFile => "fichier", "file";
    KindUrl => "lien Internet", "Internet link";
    KindSpecial => "élément système", "system item";
    TestRunning => "Analyse des icônes du bureau...", "Reading the desktop icons...";
    TestNone => "Aucune icône du bureau ne correspond.", "No desktop icon matches.";
    TestCount => "{} icônes correspondent.", "{} icons match.";
    TestNoData => "Liste des icônes indisponible : Bento Desk doit être lancé.", "Icon list unavailable: Bento Desk must be running.";
    TestHint => "Affiche les icônes actuelles du bureau qui correspondent.", "Shows the current desktop icons that match.";
    NotRunning => "Bento Desk n'est pas lancé.", "Bento Desk is not running.";
    Sorted => "Bureau trié selon les règles.", "Desktop sorted by the rules.";
    SnapSaved => "Snapshot enregistré.", "Snapshot saved.";
    SnapRestored => "Snapshot « {} » restauré.", "Snapshot \"{}\" restored.";
    SnapDeleted => "Snapshot supprimé.", "Snapshot deleted.";
    SnapFences => "{} fences", "{} fences";
    Swapped => "Fences des écrans échangées.", "Monitor fences swapped.";
    Exported => "Export enregistré.", "Export saved.";
    Imported => "Import appliqué.", "Import applied.";
    ImportFailed => "Fichier illisible : {}", "Unreadable file: {}";
    ExportFailed => "Export impossible : {}", "Export failed: {}";
    SaveFailed => "Enregistrement impossible : {}", "Cannot save: {}";
    Recovered => "Icônes ramenées sur le bureau.", "Icons brought back to the desktop.";
    ExplorerRestored => "Options de l'Explorateur rétablies.", "Explorer options restored.";
    DebugCreated => "Paquet de diagnostic créé.", "Debug package created.";
    Started => "Bento Desk démarré.", "Bento Desk started.";
    StartFailed => "Impossible de démarrer Bento Desk.", "Cannot start Bento Desk.";
    Stopped => "Bento Desk arrêté.", "Bento Desk stopped.";
    StyleAll => "Style appliqué à toutes les fences.", "Style applied to every fence.";
    ExportFilter => "Export Bento Desk (*.json)", "Bento Desk export (*.json)";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formatting() {
        assert_eq!(
            T::PageIcons.fmt(Lang::En, &["2", "14"]),
            "Page 2 · 14 icons"
        );
        assert_eq!(
            T::TestCount.fmt(Lang::Fr, &["3"]),
            "3 icônes correspondent."
        );
        assert_eq!(
            T::Portal.fmt(Lang::En, &["1", r"D:\Jeux"]),
            r"Page 1 · portal: D:\Jeux"
        );
    }
}
