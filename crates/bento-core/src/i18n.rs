//! User interface strings of the resident app, in French and English.
//!
//! The settings window has its own translations (Slint `@tr`). This table
//! only covers menus and dialogs drawn by the daemon.

use serde::{Deserialize, Serialize};

/// UI language.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Lang {
    #[default]
    Fr,
    En,
}

impl Lang {
    /// Picks a language from a Windows LANGID (primary language 0x0C = French).
    pub fn from_langid(langid: u16) -> Lang {
        if langid & 0x3FF == 0x0C {
            Lang::Fr
        } else {
            Lang::En
        }
    }
}

/// Language preference stored in the configuration.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LangPref {
    #[default]
    Auto,
    Fr,
    En,
}

impl LangPref {
    pub fn resolve(self, system_langid: u16) -> Lang {
        match self {
            LangPref::Auto => Lang::from_langid(system_langid),
            LangPref::Fr => Lang::Fr,
            LangPref::En => Lang::En,
        }
    }
}

macro_rules! messages {
    ($($id:ident => $fr:expr, $en:expr;)*) => {
        /// Identifier of a translatable string.
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub enum Msg { $($id),* }

        impl Msg {
            /// Text in the given language.
            pub fn text(self, lang: Lang) -> &'static str {
                match (self, lang) {
                    $((Msg::$id, Lang::Fr) => $fr, (Msg::$id, Lang::En) => $en,)*
                }
            }

            /// Text with `{}` replaced by `arg`.
            pub fn fmt(self, lang: Lang, arg: &str) -> String {
                self.text(lang).replacen("{}", arg, 1)
            }
        }
    };
}

messages! {
    AppName => "Bento Desk", "Bento Desk";
    TraySettings => "Paramètres", "Settings";
    TrayNewFence => "Nouveau Bento", "New Bento";
    TrayNewPortal => "Nouveau portail de dossier...", "New folder portal...";
    TrayPeek => "Peek", "Peek";
    TrayQuickHide => "Masquer ou afficher le bureau", "Hide or show the desktop";
    TraySnapshots => "Snapshots", "Snapshots";
    TraySaveSnapshot => "Enregistrer un snapshot", "Save a snapshot";
    TrayNoSnapshot => "Aucun snapshot", "No snapshot";
    TrayPause => "Mettre en pause", "Pause";
    TrayResume => "Reprendre", "Resume";
    TrayQuit => "Quitter", "Quit";
    TrayStartWithWindows => "Démarrer avec Windows", "Start with Windows";
    TraySortNow => "Trier maintenant", "Sort now";
    TrayRecoverIcons => "Récupérer toutes les icônes", "Recover all icons";
    TrayPages => "Pages de bureau", "Desktop pages";
    TrayPaused => "Bento Desk (en pause)", "Bento Desk (paused)";
    FenceRename => "Renommer", "Rename";
    FenceDelete => "Supprimer le Bento", "Delete Bento";
    FenceRollUp => "Enrouler", "Roll up";
    FenceUnroll => "Dérouler", "Unroll";
    FenceAutoRollup => "Enrouler quand ancré à un bord", "Roll up when docked to an edge";
    FenceSortBy => "Trier par", "Sort by";
    SortManual => "Manuel", "Manual";
    SortName => "Nom", "Name";
    SortType => "Type", "Type";
    SortDate => "Date de modification", "Date modified";
    SortSize => "Taille", "Size";
    SortOpens => "Nombre d'ouvertures", "Times opened";
    SortDescending => "Ordre décroissant", "Descending";
    FenceSpacing => "Ajuster l'espacement", "Adjust spacing";
    SpacingCompact => "Compact", "Compact";
    SpacingNormal => "Normal", "Normal";
    SpacingWide => "Large", "Wide";
    FenceFitContent => "Ajuster la taille au contenu", "Fit size to content";
    FenceWidth => "Largeur à l'écran", "Screen width";
    WidthFull => "Toute la largeur", "Full width";
    WidthFraction => "{} de l'écran", "{} of the screen";
    FenceCopyStyle => "Copier le style", "Copy style";
    FencePasteStyle => "Coller le style", "Paste style";
    FencePasteStyleAll => "Coller le style sur tous les Bentos", "Paste style to all Bentos";
    FenceResetStyle => "Réinitialiser le style", "Reset style";
    FenceAppearance => "Apparence...", "Appearance...";
    FenceQuickHideExempt => "Rester visible au masquage rapide", "Stay visible on quick-hide";
    FenceMoveToPage => "Déplacer vers la page", "Move to page";
    FenceLock => "Verrouiller le Bento", "Lock Bento";
    FenceOpenSettings => "Paramètres de Bento Desk...", "Bento Desk settings...";
    NewFenceTitle => "Nouveau Bento", "New Bento";
    CreateFenceHere => "Créer un Bento ici", "Create a Bento here";
    CreatePortalHere => "Créer un portail de dossier ici", "Create a folder portal here";
    Cancel => "Annuler", "Cancel";
    Ok => "OK", "OK";
    Close => "Fermer", "Close";
    DeleteFenceQuestion => "Supprimer le Bento « {} » ?", "Delete the Bento \"{}\"?";
    DeleteFenceRestore => "Supprimer et remettre les icônes sur le bureau", "Delete and put icons back on the desktop";
    DeleteFenceKeep => "Supprimer et laisser les icônes en place", "Delete and leave icons where they are";
    ResetStyleQuestion => "Réinitialiser le style de ce Bento ?", "Reset the style of this Bento?";
    Reset => "Réinitialiser", "Reset";
    RenamePrompt => "Nom du Bento", "Bento name";
    SnapshotPrompt => "Nom du snapshot", "Snapshot name";
    PortalOpen => "Ouvrir dans l'Explorateur", "Open in File Explorer";
    PortalUp => "Dossier parent", "Parent folder";
    PortalBack => "Revenir à la racine", "Back to root";
    PortalViewIcons => "Vue icônes", "Icon view";
    PortalViewList => "Vue liste", "List view";
    PortalUnavailable => "Dossier indisponible", "Folder unavailable";
    PortalRetry => "Clic pour réessayer", "Click to retry";
    PortalEmpty => "Dossier vide", "Empty folder";
    PortalChangeFolder => "Changer de dossier...", "Change folder...";
    TabDetach => "Détacher l'onglet", "Detach tab";
    TabMergeInto => "Grouper en onglets avec", "Group as tabs with";
    PageNext => "Page suivante", "Next page";
    PagePrev => "Page précédente", "Previous page";
    PageNew => "Nouvelle page", "New page";
    PageDelete => "Supprimer la page actuelle", "Delete current page";
    PageName => "Page {}", "Page {}";
    SwapScreens => "Échanger le contenu des écrans", "Swap screen contents";
    HotkeyFailed => "Raccourci indisponible : {}", "Shortcut unavailable: {}";
    PeekHint => "Échap pour fermer", "Esc to close";
    Recovered => "Icônes récupérées", "Icons recovered";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn languages() {
        assert_eq!(Lang::from_langid(0x040C), Lang::Fr);
        assert_eq!(Lang::from_langid(0x0C0C), Lang::Fr);
        assert_eq!(Lang::from_langid(0x0409), Lang::En);
        assert_eq!(LangPref::En.resolve(0x040C), Lang::En);
        assert_eq!(Msg::TrayQuit.text(Lang::En), "Quit");
        assert_eq!(Msg::PageName.fmt(Lang::Fr, "2"), "Page 2");
    }
}
