# Bento Desk - Étude technique et architecture

Statut : choix validés le 2026-11-25 (section 12).
Date : 2026-11-25.
Portée : remplaçant open source (MIT) de Stardock Fences 6, écrit en Rust, sans aucune injection dans d'autres processus.

---

## 0. Résumé des décisions

| Sujet | Décision proposée |
|---|---|
| Fonds des fences | Fenêtres de l'app, enfants de la couche bureau d'explorer, placées **sous** `SHELLDLL_DefView` (méthode utilisée par Lively et Wallpaper Engine). |
| Cadre et titre des fences | Une seconde fenêtre par fence, placée **au-dessus** de `SHELLDLL_DefView`. Son corps est transparent (alpha 0), donc les clics y traversent jusqu'aux icônes. |
| Icônes | Vraies icônes du bureau, positionnées par `IFolderView::SelectAndPositionItems` (API documentée et supportée). |
| Icônes masquées (défilement, roll-up, onglets, pages) | « Parking » des icônes hors de l'écran virtuel. |
| Portails de dossiers et Peek | Rendu propre à l'app (icônes du shell dessinées par Bento Desk). |
| Gestes sur le bureau | Hook `WH_MOUSE_LL` installé **uniquement** quand le bureau est au premier plan. Il est retiré dès qu'une autre fenêtre passe au premier plan. |
| Rendu | Direct2D en mode logiciel et DirectWrite, présentés par `UpdateLayeredWindow`. Pas de device D3D résident. |
| Paramètres | Processus séparé `bento-settings.exe`, lancé à la demande, en **Slint** (style Fluent). |
| Démarrage | Clé `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`. |
| Cible de test | Windows 10 22H2. Windows 11 24H2+ est pris en charge dans le code, mais non testé (section 2.2). |
| Livrables binaires | Deux `.exe`, aucune DLL, aucun serveur COM enregistré. |

---

## 1. Pourquoi Fences pose problème, et ce que Bento Desk garantit

`FencesMenu64.dll` est une **extension shell** (gestionnaire de menu contextuel). Une extension shell est un serveur COM *in-process*. Windows la charge dans **tout processus** qui affiche un menu contextuel du shell, un dialogue Ouvrir/Enregistrer ou qui énumère des verbes de fichiers. Le jeu ou son launcher peut donc charger la DLL de Fences. Vanguard voit alors un module tiers dans un processus protégé.

Bento Desk applique une règle stricte : **aucun code de Bento Desk ne s'exécute dans un autre processus**.

Interdit, et vérifié en CI (section 11) :
- extensions shell (`IContextMenu`, `IShellExtInit`, overlays, handlers de propriétés, etc.) et tout serveur COM enregistré ;
- DLL injectée, `CreateRemoteThread`, `VirtualAllocEx`, `WriteProcessMemory`, Detours, MinHook ;
- `SetWindowsHookEx` avec un type autre que `WH_MOUSE_LL` (pas de hook clavier, même LL) ;
- exécution en administrateur (`asInvoker`).

Autorisé, car aucun code de l'app n'est chargé ailleurs :
- messages Win32 envoyés à des fenêtres d'explorer (`SendMessageTimeout`, `PostMessage`) ;
- `SetParent` d'une fenêtre de l'app sous une fenêtre d'explorer ;
- appels COM inter-processus vers explorer (`IShellWindows`, `IFolderView`). Le code exécuté côté explorer est celui de Windows ;
- `SetWinEventHook` avec `WINEVENT_OUTOFCONTEXT` : les événements sont livrés dans le processus de l'app, sans DLL chargée ailleurs ;
- OLE drag and drop : le transfert passe par RPC.

Point à noter : quand un portail de dossier affiche un menu contextuel du shell, les extensions tierces se chargent **dans le processus de Bento Desk**. Cela ne concerne aucun autre processus.

---

## 2. La couche bureau

### 2.1 Windows 10 22H2 et Windows 11 jusqu'à 23H2

Structure par défaut : `Progman` > `SHELLDLL_DefView` > `SysListView32` (la liste d'icônes).

Le message non documenté `0x052C` envoyé à `Progman` (avec `SendMessageTimeout`) crée un `WorkerW` de niveau supérieur, situé derrière le `WorkerW` qui contient `SHELLDLL_DefView`. Les fonds d'écran animés (Wallpaper Engine, Lively) dessinent dans ce `WorkerW` arrière. Les icônes restent dessinées par-dessus, avec un fond transparent.

Bento Desk utilise la même méthode : le fond de chaque fence est un enfant de ce `WorkerW` arrière.

### 2.2 Windows 11 24H2 et suivants (build 26100+)

Changement de structure : `SHELLDLL_DefView` et le `WorkerW` sont désormais des **enfants de `Progman`**. Le code qui cherche le `WorkerW` comme fenêtre de niveau supérieur ne trouve plus rien.

Méthode retenue, d'après les correctifs publiés pour Lively et d'autres projets :
1. Envoyer `0x052C` à `Progman` avec wParam `0xD` et lParam `1`, pour que `Progman` crée le `WorkerW` comme enfant.
2. Créer le fond de la fence comme enfant de `Progman` (ou du `WorkerW` enfant), avec le style `WS_EX_LAYERED`. Sur 24H2, un enfant de `Progman` sans ce style n'est pas dessiné.
3. Le placer dans l'ordre Z **juste sous** `SHELLDLL_DefView`.

Sources :
- Lively issue #2074 : https://github.com/lively-community/lively/issues/2074
- Lively issue #2695 : https://github.com/rocksdanister/lively/issues/2695
- kirie PR #11 : https://github.com/UnhingedSoftware/kirie/pull/11
- beautiful-wallpaper PR #83 : https://github.com/C0uki/beautiful-wallpaper/pull/83
- Article dev.to sur les changements de 24H2 : https://dev.to/abdelhamid_qaiss_16bb6fd9/how-i-put-a-video-wallpaper-behind-the-windows-11-desktop-icons-and-what-changed-in-24h2-56j0
- Microsoft a posé un blocage de mise à jour 24H2 pour les apps de personnalisation de fond d'écran : https://www.theregister.com/2025/04/14/windows_11_24h2_hold/

Niveau de prise en charge : l'app est développée et testée sur Windows 10 22H2. Le chemin 24H2 est codé d'après les sources ci-dessus, sans test réel. Un défaut sur Windows 11 sera corrigé plus tard, quand il apparaîtra.

Détection de la variante : l'app ne se fie pas au numéro de build. Elle inspecte l'arbre des fenêtres après `0x052C` (où se trouve `SHELLDLL_DefView`, où se trouve le `WorkerW`). Une future version de Windows qui garde l'une des deux structures fonctionnera sans modification.

### 2.3 Une fence = deux fenêtres

| Fenêtre | Parent et ordre Z | Rôle | Entrée souris |
|---|---|---|---|
| `FenceBackdrop` | Sous `SHELLDLL_DefView` | Fond coloré, flou, bordure | Aucune : la liste d'icônes est au-dessus. |
| `FenceFrame` | Au-dessus de `SHELLDLL_DefView`, même parent | Barre de titre, onglets, bords de redimensionnement, bouton de défilement | Native sur les pixels visibles. Les pixels d'alpha 0 (le corps) laissent passer les clics vers les icônes. |

Une fenêtre layered présentée par `UpdateLayeredWindow` ne reçoit pas les clics sur ses pixels d'alpha 0. Le corps du `FenceFrame` est donc « troué » : les icônes y restent pleinement natives (double-clic, menu contextuel, sélection multiple, glisser-déposer). Ce comportement est à confirmer pour une fenêtre enfant (spike S2).

### 2.4 Points d'attention

- **`SetParent` entre processus** : Windows attache la file d'entrée du thread de l'app à celle du thread du bureau d'explorer. Si le thread UI de l'app bloque, le bureau bloque aussi. Mitigation : le thread UI ne fait jamais d'appel bloquant. Les appels COM vers explorer passent sur un thread dédié (section 3.3).
- **Redémarrage d'explorer** : les fenêtres enfants de l'app sont détruites avec leur parent. L'app écoute le message enregistré `TaskbarCreated`, puis reconstruit la couche bureau, les fenêtres et la connexion COM. Tous les handles sont des types RAII, donc rien ne fuit.
- **Explorer pas encore prêt au login** : l'app attend `TaskbarCreated` au lieu d'interroger en boucle.
- **Coexistence avec Wallpaper Engine ou Lively** : ils utilisent le même `WorkerW`. Les fences doivent rester au-dessus de leurs fenêtres dans l'ordre Z. Si un de ces logiciels recrée son `WorkerW`, l'app doit se réattacher (risque R4).
- **« Afficher les icônes du bureau » désactivé par l'utilisateur** : `SysListView32` est masquée. Les fences se masquent aussi.

---

## 3. Lire et positionner les vraies icônes

### 3.1 Chaîne COM documentée

Méthode de Raymond Chen (« Manipulating the positions of desktop icons », https://devblogs.microsoft.com/oldnewthing/20130318-00/?p=4933) :

1. `CoCreateInstance(CLSID_ShellWindows)` puis `IShellWindows::FindWindowSW(CSIDL_DESKTOP, SWC_DESKTOP, SWFO_NEEDDISPATCH)`.
2. `IServiceProvider::QueryService(SID_STopLevelBrowser)` donne `IShellBrowser`.
3. `IShellBrowser::QueryActiveShellView` donne `IShellView`, puis `IFolderView2`.

Opérations :
- `IFolderView::Items(SVGIO_ALLVIEW)` et `GetItemPosition` : lecture des positions.
- `IFolderView::SelectAndPositionItems(SVSI_POSITIONITEM)` : écriture. Doc : https://learn.microsoft.com/windows/win32/api/shobjidl_core/nf-shobjidl_core-ifolderview-selectandpositionitems
- `IFolderView::GetSpacing` et `IFolderView2::GetViewModeAndIconSize` : taille de la grille et des icônes, pour aligner les icônes dans les fences et faire nos propres tests de position (hit test).
- `IFolderView2::SetCurrentFolderFlags` : désactivation de `FWF_AUTOARRANGE` et `FWF_SNAPTOGRID`. Sans cela, explorer replace les icônes. Bento Desk mémorise les valeurs d'origine et les restaure en option à la désinstallation.

Depuis Windows 10 1809, envoyer des messages directement à la `SysListView32` ne déplace plus correctement les icônes. `SelectAndPositionItems` est la seule voie supportée. Source : https://devblogs.microsoft.com/oldnewthing/20211122-00/?p=105948

### 3.2 Identité des icônes

- Clé stable : le nom de parsing (`SHGDN_FORPARSING`). Le bureau fusionne le dossier Bureau de l'utilisateur, le Bureau public et des éléments virtuels (Corbeille, Ce PC). Le nom de parsing distingue ces cas.
- Renommage : la notification `SHCNE_RENAMEITEM` donne l'ancien et le nouveau PIDL. L'appartenance à une fence suit l'élément.
- Les positions de la listview sont en pixels physiques, avec l'origine au coin supérieur gauche de l'écran virtuel. La conversion vers les coordonnées de l'écran est testée unitairement.

### 3.3 Threads

| Thread | Rôle | Au repos |
|---|---|---|
| UI | Boucle de messages, fenêtres, rendu, tray, raccourcis | Bloqué dans `GetMessage` |
| Shell (STA COM) | Tous les appels `IFolderView` vers explorer. Ils sont synchrones et inter-processus, donc un explorer figé ne bloque que ce thread. | Bloqué dans `GetMessage` |
| Input | Héberge le hook `WH_MOUSE_LL` quand il est actif. Il fait le minimum et poste un message au thread UI. Ainsi, le hook répond vite même pendant un rendu. | Bloqué dans `GetMessage`, hook absent |
| Log | Écriture non bloquante (`tracing-appender`) | Bloqué |
| Tâches ponctuelles | Énumération des portails, chargement d'icônes | Inexistant (créé à la demande) |

Windows retire silencieusement un hook LL qui dépasse `LowLevelHooksTimeout`. C'est pour cela que le hook a son propre thread.

### 3.4 Surveillance des changements (sans polling)

| Événement | Mécanisme |
|---|---|
| Ajout, suppression, renommage d'icône | `ReadDirectoryChangesW` sur le Bureau utilisateur et le Bureau public, un thread bloquant par dossier (voir section 16) |
| Redémarrage d'explorer | Message `TaskbarCreated` |
| Résolution, écrans, DPI | `WM_DISPLAYCHANGE`, `WM_DPICHANGED`, `WM_SETTINGCHANGE` (zone de travail). Une fenêtre cachée de niveau supérieur les reçoit : une fenêtre `HWND_MESSAGE` ne reçoit pas les messages diffusés. |
| Sortie de veille | `WM_POWERBROADCAST` (`PBT_APMRESUMEAUTOMATIC`) |
| Écrans qui s'allument en retard | Minuterie ponctuelle de 1,5 s après le dernier changement d'affichage (anti-rebond, puis plus de timer) |
| Fond d'écran (Chameleon, flou) | `WM_SETTINGCHANGE` avec `SPI_SETDESKWALLPAPER` |
| Thème clair/sombre | `WM_SETTINGCHANGE` « ImmersiveColorSet » |
| Premier plan (activation du hook, mode jeu) | `SetWinEventHook(EVENT_SYSTEM_FOREGROUND, WINEVENT_OUTOFCONTEXT)` |
| Fin d'un glisser d'icône | Bouton relâché vu par le hook, puis lecture des positions 100 ms plus tard (minuterie ponctuelle) et calcul du diff |

---

## 4. Entrée souris sans injection

### 4.1 Le problème

La `SysListView32` du bureau couvre tout l'écran. Elle reçoit tous les clics hors des zones visibles des `FenceFrame`. Certaines fonctionnalités de Fences dépendent pourtant de gestes faits sur le bureau lui-même :
- clic droit + glisser pour dessiner une fence ;
- double-clic sur une zone vide pour le quick-hide ;
- molette dans le corps d'une fence pour défiler ;
- double-clic sur une icône pour compter les ouvertures.

### 4.2 La solution : un hook LL à durée de vie courte

1. `SetWinEventHook(EVENT_SYSTEM_FOREGROUND, WINEVENT_OUTOFCONTEXT)` signale chaque changement de fenêtre au premier plan.
2. Si la fenêtre au premier plan est le bureau (`Progman`, `WorkerW` ou une fenêtre de l'app), le thread Input installe `WH_MOUSE_LL`.
3. Dès qu'une autre fenêtre passe au premier plan, le hook est retiré.

Conséquences :
- Quand un jeu est au premier plan, Bento Desk n'a **aucun hook actif** et ne reçoit aucun événement souris.
- Quand le bureau est au premier plan, aucun jeu ne reçoit d'entrée, donc il n'y a aucune latence ajoutée en jeu.
- Le hook n'avale que deux types d'événements : le clic droit commencé sur une zone vide (pour dessiner une fence) et la molette dans le corps d'une fence. Tout le reste passe immédiatement.

Création d'une fence au clic droit + glisser :
1. Le clic droit enfoncé sur une zone vide est avalé.
2. L'app dessine le rectangle dans sa propre fenêtre temporaire.
3. Si la souris n'a presque pas bougé au relâchement, l'app poste `WM_RBUTTONDOWN` puis `WM_RBUTTONUP` à la `SysListView32`. Le menu natif du bureau s'affiche alors normalement (spike S6).

Aucune source consultée n'indique que Vanguard réagit aux hooks `WH_MOUSE_LL`. Le hook étant absent pendant le jeu, le risque est faible. Il reste à tester (section 13).

### 4.3 Mode jeu

Si l'app au premier plan est en plein écran (`SHQueryUserNotificationState` renvoie `QUNS_RUNNING_D3D_FULL_SCREEN` ou `QUNS_BUSY`), ou si son exécutable figure dans une liste d'exclusion (League of Legends et Valorant par défaut), Bento Desk :
- retire le hook s'il était actif ;
- annule les minuteries non essentielles ;
- n'exécute plus aucun rendu jusqu'au retour du bureau.

### 4.4 Bords d'écran (pages de bureau)

Le changement de page en poussant la souris contre un bord n'utilise ni hook ni polling. Des bandes de 1 px, possédées par l'app, sont placées dans la couche bureau. Elles sont quasi transparentes (alpha 1) mais reçoivent `WM_MOUSEMOVE`. Limite : elles ne réagissent que là où le bureau est visible.

---

## 5. Solutions hybrides (à valider avant de coder)

### 5.1 Le parking des icônes

Les icônes réelles appartiennent à une seule liste qui couvre l'écran. L'app ne peut ni les rogner ni masquer une icône précise. Fences 6 propose pourtant des fonctionnalités qui exigent de cacher une partie des icônes :
- défilement d'une fence trop petite ;
- roll-up ;
- onglets ;
- pages de bureau ;
- quick-hide avec exclusions.

Solution : les icônes à cacher sont déplacées hors de l'écran virtuel (« parking ») avec `SelectAndPositionItems`. Elles reviennent à leur place quand elles doivent réapparaître.

Conséquences :
- Défilement **ligne par ligne**, pas fluide au pixel. Chaque cran de molette replace une ligne d'icônes.
- Un déplacement de plusieurs dizaines d'icônes produit un bref redessin de la liste. Le coût est à mesurer (spike S3).
- Si l'app est tuée brutalement, des icônes peuvent rester parquées. Au prochain démarrage, l'app les restaure. Un menu « Récupérer toutes les icônes » est aussi prévu dans la section Dépannage.
- Il reste à vérifier qu'explorer conserve des positions hors écran après un redémarrage (spike S3). Sinon, l'app réapplique sa disposition après `TaskbarCreated`.

### 5.2 Rendu propre à l'app : portails de dossiers et Peek

Les portails affichent des éléments qui ne sont pas sur le bureau. Ils ne peuvent donc pas être de vraies icônes du bureau. Bento Desk les dessine lui-même :
- énumération avec `IShellFolder` et `IShellItem` sur un thread ponctuel ;
- délai maximal pour les dossiers réseau, avec affichage « Dossier indisponible » et bouton Réessayer ;
- icônes et vignettes via `IShellItemImageFactory` ;
- ouverture via `ShellExecuteEx`. « Ouvrir » lance une nouvelle fenêtre de l'Explorateur ;
- menu contextuel via `IContextMenu` dans le processus de l'app ;
- glisser-déposer via `SHCreateDataObject` et `DoDragDrop` ;
- chemins longs : API `IShellItem` et manifeste `longPathAware` ;
- surveillance du dossier : `ReadDirectoryChangesW` tant que le portail existe (voir section 16).

**Peek** : les vraies icônes ne peuvent pas passer au-dessus des fenêtres. Peek ouvre donc une surcouche topmost qui redessine les fences et leurs icônes avec le même composant que les portails. Le fond utilise le backdrop Acrylic de DWM (Windows 11 22H2+), avec un voile semi-transparent en repli sur Windows 10. La sélection multiple avec Ctrl est gérée par l'app. La surcouche tient compte de la zone de travail de chaque écran (`rcWork`), donc des barres des tâches à gauche ou à droite.

### 5.3 Dépôt d'une icône du bureau sur un onglet

L'onglet appartient au `FenceFrame`, qui est enregistré comme `IDropTarget`. À la dépose, l'app identifie l'élément, l'affecte à la fence de l'onglet et répond `DROPEFFECT_NONE`. La source ne fait donc aucune action sur le fichier (pas de suppression après déplacement).

---

## 6. Fonctionnalités impossibles ou dégradées

| Fonctionnalité | Statut | Raison | Alternative |
|---|---|---|---|
| Entrée « Configurer » dans le menu clic droit du bureau | Impossible | Exige une extension shell, donc du code chargé dans explorer et dans tout processus qui affiche un menu du shell. C'est exactement le défaut de Fences. | Icône dans la zone de notification. Clic droit sur la barre de titre d'une fence. |
| Menu Fences au clic droit dans le corps d'une fence | Dégradé | Le corps appartient à la liste d'icônes d'explorer. | Menu de la fence sur sa barre de titre (clic droit ou bouton). Le corps garde le menu natif du bureau. |
| Titre « au survol » du corps | Dégradé | Le survol du corps n'est visible que par le hook, actif seulement quand le bureau est au premier plan. | Le survol de la barre de titre fonctionne toujours. |
| Défilement fluide | Dégradé | Les icônes réelles ne peuvent pas être rognées. | Défilement ligne par ligne (parking). |
| Teinte d'icônes (icon tint) | Abandonnée (décision) | Les vraies icônes sont dessinées par explorer. Les modifier exige une injection. | Aucune. La fonctionnalité est retirée du périmètre, y compris pour les portails et Peek. |
| Remplacer Win+D (« Desktop Display ») | Impossible | Win+D est un raccourci réservé au shell. Le remplacer exige un hook clavier global, exclu. | Les fences restent visibles après Win+D, car elles sont dans la couche bureau (à confirmer, spike S2). Raccourci configurable pour l'action de l'app. |
| Peek avec les vraies icônes au-dessus des fenêtres | Dégradé | Les vraies icônes vivent sous toutes les fenêtres. | Surcouche dessinée par l'app (section 5.2). |
| Peek sur Win+Espace | Incertain | Microsoft réserve les raccourcis avec la touche Windows. Win+Espace change la disposition du clavier. Doc : https://learn.microsoft.com/windows/win32/api/winuser/nf-winuser-registerhotkey | Tenter `RegisterHotKey`. Si plusieurs dispositions sont installées (`GetKeyboardLayoutList`) ou si l'enregistrement échoue, proposer Ctrl+Win+Espace et l'indiquer clairement dans les paramètres. Pas de hook clavier. |
| Tri par nombre d'ouvertures | Dégradé | Explorer ne publie pas les lancements depuis le bureau. | Comptage des double-clics vus par le hook. Les lancements par la touche Entrée ne sont pas comptés. UserAssist n'est pas utilisé (décision). |
| Acrylic/Mica sur les fonds de fences | Dégradé | Les backdrops DWM ne s'appliquent qu'aux fenêtres de niveau supérieur, pas aux enfants de la couche bureau. | Flou simulé : l'app floute elle-même la portion du fond d'écran sous la fence (recalcul seulement au changement de fond d'écran). Vrai Mica/Acrylic dans les paramètres, les menus et Peek. |
| Quick-hide global | Réalisable avec réserve | `IFolderView2::SetCurrentFolderFlags(FWF_NOICONS)` passe par une interface officielle, mais l'usage n'est pas documenté par Microsoft. | Avec exclusions : parking des icônes concernées. |

---

## 7. Matrice de parité avec Fences 6

Légende : **N** = natif (icônes réelles), **H** = hybride (parking ou rendu propre), **D** = dégradé (section 6).

| # | Fonctionnalité | Statut | Mécanisme principal | Phase |
|---|---|---|---|---|
| 1 | Création au clic droit + glisser, et depuis le menu | N | Hook LL (bureau au premier plan) | 1 |
| 1 | Renommer, déplacer, redimensionner, supprimer (avec retour des icônes sur le bureau) | N | `FenceFrame` | 1 |
| 1 | Magnétisme, alignement, « Ajuster l'espacement » | N | Calcul dans `bento-core` | 2 |
| 1 | Glisser-déposer entre fences et bureau, sélection multiple | N | Liste d'icônes native et diff des positions | 1 |
| 1 | Défilement | H, D | Parking ligne par ligne | 2 |
| 1 | Tri (nom, type, date, taille, ouvertures) | N, D pour ouvertures | Propriétés shell | 3 |
| 2 | Couleur, transparence, intensité, teinte, saturation | N | Rendu du `FenceBackdrop` | 2 |
| 2 | Titre jamais, au survol, toujours ; police et couleur | N, D (survol du corps) | `FenceFrame` | 2 |
| 2 | Acrylic, Mica, flou | D | Flou simulé ; repli couleur unie sur Windows 10 | 2 |
| 2 | Chameleon | N | Couleur moyenne du fond d'écran sous la fence | 5 |
| 2 | Teinte d'icônes | Abandonnée | Voir section 6 | - |
| 2 | Copier/coller le style, réinitialiser avec confirmation | N | Logique de domaine | 2 |
| 3 | Onglets (regrouper, déposer sur un onglet, couleur, détacher) | H | Parking et `IDropTarget` | 5 |
| 4 | Portails de dossiers | H | Rendu propre | 3 |
| 5 | Roll-up (double-clic, ouverture au clic ou au survol, ancrage, reste ouvert pendant un glisser ou un menu) | H | Parking ; survol natif sur le titre | 2 |
| 6 | Quick-hide (double-clic, exclusions, inactivité) | H | `FWF_NOICONS` ou parking ; minuterie ponctuelle réarmée à chaque entrée | 2 |
| 7 | Règles et tri automatique, éditeur, test, « Trier maintenant » | N | `bento-core` et `ReadDirectoryChangesW` | 3 |
| 8 | Peek | H, D | Surcouche propre | 5 |
| 9 | Pages de bureau (raccourci, bords, indicateur) | H | Parking ; bandes de bord | 5 |
| 10 | Snapshots nommés | N | `bento-config` | 4 |
| 10 | Layout automatique par configuration d'écrans | N | `QueryDisplayConfig` et chemins de périphérique des écrans | 4 |
| 10 | Échanger le contenu des écrans | N | Calcul dans `bento-core` | 4 |
| 11 | Fences par écran, DPI par écran jusqu'à 400 % | N | Per-Monitor V2 | 4 (base en phase 1) |
| 12 | Tray, paramètres, tâche en cours, export/import, thème, FR/EN, instance unique | N | Voir section 8 | 1 et 6 |

Remarque sur la phase 1 : le geste clic droit + glisser exige le hook souris. Le hook et le geste sont donc livrés dès la phase 1 (décision). Le quick-hide, qui réutilise le même hook, reste en phase 2.

---

## 8. Application

### 8.1 Organisation du workspace Cargo

```
bento-desk/
  Cargo.toml                (workspace, profils, lints communs)
  crates/
    bento-core/             domaine pur, 100 % safe, sans dépendance Windows
    bento-config/           persistance JSON, schéma versionné, migrations
    bento-win/              seul crate contenant du code unsafe : interop Win32/COM
    bento-render/           rendu Direct2D/DirectWrite (s'appuie sur bento-win)
  apps/
    bento-desk/             daemon résident (bento-desk.exe)
    bento-settings/         fenêtre de paramètres (bento-settings.exe)
  docs/
  installer/                script Inno Setup
  .github/workflows/        CI
```

| Crate | Contenu | Tests |
|---|---|---|
| `bento-core` | Modèles (fence, onglet, page, règle, layout), moteur de règles, calcul de grille et d'espacement, magnétisme, conversions DIP et pixels, clés de configuration d'écrans, tri, logique du parking | Unitaires, sans Windows |
| `bento-config` | `serde` et `serde_json`, fichiers `config.json`, `layouts.json`, `snapshots/`, `rules.json`. Champ `schema_version` et migrations successives v1 vers v2, etc. Écriture atomique (fichier temporaire puis `ReplaceFileW`). Export et import. | Sérialisation aller-retour, migrations, fichiers corrompus |
| `bento-win` | Types RAII : `OwnedHwnd`, `OwnedHandle`, `OwnedHdc`, `OwnedHbitmap`, `ComApartment`, `WinEventHook`, `MouseLlHook`, `HotKey`, `NotifyIcon`, `ChangeNotifyRegistration`. Couche bureau, accès à `IFolderView`, DWM, affichage, DPI, raccourcis. Chaque bloc `unsafe` porte un commentaire de justification avec le lien Microsoft Learn. | Tests d'intégration marqués `#[ignore]` (exigent une session interactive) |
| `bento-render` | Fond, cadre, titre, onglets, vue d'icônes des portails et de Peek, menus et popups au style Fluent | Tests de mise en page (calculs, sans rendu) |
| `apps/bento-desk` | `main`, instance unique, boucle de messages, threads, tray, orchestration, journaux, `anyhow` | - |
| `apps/bento-settings` | Interface des paramètres | - |

Règles transverses :
- `#![forbid(unsafe_code)]` dans tous les crates sauf `bento-win`.
- Lints du workspace : `clippy::unwrap_used` et `clippy::expect_used` refusés hors tests. `unsafe_op_in_unsafe_fn` refusé.
- Erreurs : `thiserror` dans les crates de bibliothèque, `anyhow` seulement dans `apps/*`.

### 8.2 Dépendances externes

| Crate | Où | Justification |
|---|---|---|
| `windows` | `bento-win`, `bento-render` | Imposé. Bindings officiels Microsoft. Fonctionnalités activées au plus juste pour réduire le temps de compilation. |
| `serde`, `serde_json` | `bento-config`, `bento-core` | Imposé. |
| `thiserror` | bibliothèques | Imposé. |
| `anyhow` | binaires | Imposé. |
| `tracing`, `tracing-subscriber` (fonctionnalité `reload`), `tracing-appender` | binaires | Imposé. `reload` permet de changer le niveau de log sans redémarrer. `tracing-appender` fournit le fichier roulant. |
| `slint` | `bento-settings` seulement | Si l'option est validée (section 9). |

Aucune autre dépendance. Fonctions écrites à la main plutôt qu'ajoutées en crate :
- dossier AppData via `SHGetKnownFolderPath` ;
- icône de notification via `Shell_NotifyIconW` ;
- traductions du daemon : une petite table de chaînes FR/EN dans `bento-core`.

### 8.3 Rendu

- **Direct2D en mode logiciel** (`D2D1_RENDER_TARGET_TYPE_SOFTWARE`) dans une DIB 32 bits, puis `UpdateLayeredWindow` avec alpha par pixel.
  - Même chemin sur Windows 10 et 11, compatible avec l'exigence `WS_EX_LAYERED` de 24H2.
  - Aucun device D3D ni pilote GPU chargé en permanence. Un device D3D coûte souvent 10 à 30 Mo de mémoire.
  - Les fences ne sont redessinées que sur événement (redimensionnement, survol, changement de style). Le coût CPU est donc négligeable.
- **DirectWrite** pour le texte (titres, onglets, noms dans les portails), avec la police Segoe UI Variable par défaut sous Windows 11.
- **Flou** : la portion du fond d'écran sous chaque fence est lue avec `IDesktopWallpaper` et WIC, réduite, puis floutée (trois passes de flou boîte, approximation d'un flou gaussien). Le résultat est mis en cache. Recalcul seulement quand le fond d'écran, la position ou la taille changent.
- **Look 2026** : coins arrondis, ombres douces, Segoe UI Variable, couleur d'accent Windows, animations courtes à l'ouverture et à la fermeture du roll-up. Les animations sont cadencées sur le rafraîchissement de l'écran : un thread attend chaque passe de composition du DWM (`DwmFlush`) et poste un message par image tant qu'une animation tourne, puis se met en pause. Pendant l'animation, les surfaces de dessin sont allouées une fois à la hauteur ouverte et réutilisées à chaque image.
- **Menus** (fences, tray) : popups dessinés par l'app au style Fluent, clair ou sombre. Sur Windows 11, coins arrondis (`DWMWA_WINDOW_CORNER_PREFERENCE`) et backdrop Acrylic (`DWMWA_SYSTEMBACKDROP_TYPE`). Ce choix évite l'API non documentée de mode sombre des menus Win32.
- DirectComposition n'est pas nécessaire avec ce choix.

### 8.4 DPI et multi-écrans

- Manifeste Per-Monitor V2. Les tailles des fences sont stockées en DIP, avec l'identifiant de l'écran.
- Les positions d'icônes sont stockées comme un index dans la grille de la fence, et non en pixels. Un changement de DPI recalcule donc simplement la grille.
- Identifiant stable d'écran : chemin de périphérique du moniteur (`DisplayConfigGetDeviceInfo`, `DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME`).
- Clé d'une configuration d'écrans : ensemble trié des chemins de moniteurs avec résolution, position et DPI. Une nouvelle clé crée un layout. Une clé connue restaure le sien.

### 8.5 Instance unique, tray, démarrage

- Mutex nommé `Local\BentoDesk.SingleInstance`.
  - Un second lancement trouve la fenêtre cachée du daemon (classe connue), lui envoie un message enregistré « ouvrir les paramètres », puis se ferme.
  - Le daemon lance `bento-settings.exe` s'il ne tourne pas déjà. Les paramètres ont leur propre mutex.
- Tray : ouvrir les paramètres, nouvelle fence, Peek, quick-hide, snapshots, pause, quitter.
- « Afficher comme tâche en cours » : une fenêtre de niveau supérieur sans `WS_EX_TOOLWINDOW`, qui donne un bouton dans la barre des tâches.
- Démarrage avec Windows : clé `HKCU\...\Run`, écrite et supprimée par la case à cocher des paramètres.
  - Limite connue : Windows retarde de quelques secondes les programmes de la clé `Run` après l'ouverture de session. Ce délai est accepté (décision).
- Thème clair ou sombre : valeur `AppsUseLightTheme` de `HKCU\Software\Microsoft\Windows\CurrentVersion\Themes\Personalize`, relue à chaque `WM_SETTINGCHANGE`.
- Langue : `GetUserDefaultUILanguage`, avec un choix manuel FR ou EN dans les paramètres.

### 8.6 Communication entre le daemon et les paramètres

- Les paramètres écrivent les fichiers JSON (écriture atomique), puis postent un message enregistré `BentoDesk.ConfigChanged` à la fenêtre du daemon.
- Le daemon recharge la configuration après un anti-rebond de 100 ms. C'est ce qui permet l'aperçu en direct des couleurs.
- Les paramètres qui ont besoin de l'état courant (liste des icônes pour tester une règle) le lisent dans un fichier d'état que le daemon écrit à la demande.

### 8.7 Journaux

- `%LOCALAPPDATA%\BentoDesk\logs\`, rotation journalière, 7 fichiers conservés.
- Niveau réglable dans les paramètres, appliqué à chaud.
- « Paquet de debug » dans Dépannage : archive zip avec les journaux, la configuration et un relevé de l'arbre des fenêtres du bureau, sans donnée personnelle hors noms d'icônes. La création du zip utilise l'API shell (`IShellDispatch`, `CopyHere` vers un dossier compressé), donc aucun crate.

---

## 9. Fenêtre de paramètres : comparaison

### 9.1 Principe commun

Les paramètres tournent dans un **processus séparé**. Il est lancé à la demande et se termine à la fermeture de la fenêtre. Le daemon ne charge jamais de bibliothèque d'interface. Sa mémoire au repos ne dépend donc pas de ce choix.

### 9.2 Comparaison

Les chiffres sont des **estimations**. Elles seront mesurées avec trois « hello world » au début de la phase 1, avant de figer le choix.

| Critère | Win32 natif | Slint | egui (eframe) |
|---|---|---|---|
| RAM, fenêtre ouverte | 6 à 12 Mo | 25 à 45 Mo | 40 à 90 Mo (wgpu) ou 30 à 50 Mo (glow) |
| RAM au repos (fenêtre fermée) | 0 (processus terminé) | 0 | 0 |
| Taille ajoutée au binaire | Moins de 0,5 Mo | 4 à 7 Mo | 3 à 6 Mo |
| Look 2026 sur Windows 11 | Faible : contrôles communs v6 des années 2010. Mode sombre via une API non documentée (`uxtheme`). | Bon : style `fluent` natif, clair et sombre automatiques, Mica applicable au HWND. | Moyen : style propre à egui, à personnaliser fortement. |
| Rendu Windows 10 | Bon | Bon | Bon |
| Accessibilité | Excellente (UIA natif) | Bonne (AccessKit vers UIA) | Correcte (AccessKit) |
| Licence et compatibilité MIT | Aucune contrainte | GPLv3, ou Royalty-free 2.0 compatible MIT avec attribution obligatoire (widget `AboutSlint` ou badge sur la page de téléchargement) | MIT ou Apache-2.0, aucune contrainte |
| Effort pour 12 sections | Élevé (mise en page manuelle, contrôles personnalisés pour les couleurs) | Moyen (langage déclaratif, widgets Fluent prêts) | Moyen (mode immédiat, mise en page simple) |
| Traduction FR/EN | Table de chaînes maison | `@tr` intégré (gettext) | Maison |

Option écartée : **WinUI 3 / Windows App SDK**. Elle exige un runtime à installer, consomme environ 100 Mo de RAM une fois ouverte, et ses bindings Rust ne sont pas officiels.

Sources pour la licence Slint :
- https://slint.dev/agreements/slint-royalty-free-license.pdf
- https://raw.githubusercontent.com/slint-ui/slint/HEAD/FAQ.md

### 9.3 Recommandation : Slint

Raisons :
1. C'est la seule option qui donne un look Windows 11 « 2026 » sans effort démesuré (style `fluent`, thème suivant Windows, Mica sur la fenêtre).
2. Accessibilité correcte via AccessKit.
3. Le coût en RAM n'existe que pendant que la fenêtre est ouverte, grâce au processus séparé.
4. La licence Royalty-free 2.0 est compatible avec le code MIT du projet. Seule contrainte : afficher `AboutSlint` dans la section À propos.

Alternative si une licence 100 % permissive est préférée : **egui**, au prix d'un look moins natif.

**Choix validé : Slint.**

---

## 10. Build, manifeste, packaging

### 10.1 Profil release

```toml
[profile.release]
lto = true
codegen-units = 1
opt-level = "s"
panic = "abort"
strip = "symbols"
debug = false
```

- `opt-level = "s"` : le daemon passe l'essentiel de son temps bloqué dans `GetMessage`. La vitesse brute compte peu. Un binaire plus petit réduit la mémoire mappée et le temps de chargement au démarrage. Le rendu logiciel reste rapide, car il n'a lieu que sur événement. À reconsidérer seulement si une mesure montre un rendu lent.
- `panic = "abort"` : pas de tables de déroulement de pile, binaire plus petit. Un hook de panique écrit le message et la pile dans le journal avant l'arrêt. Le code ne doit de toute façon pas paniquer (pas de `unwrap` ni d'`expect`).
- `strip` : symboles retirés du binaire publié. Un build séparé avec PDB est conservé en artefact CI pour analyser les crashs.

### 10.2 Manifeste d'application (embarqué par `build.rs` via le compilateur de ressources)

- `dpiAwareness` = `PerMonitorV2`, `dpiAware` = `true/pm` pour la compatibilité ;
- contrôles communs v6 (`Microsoft.Windows.Common-Controls`) ;
- `requestedExecutionLevel level="asInvoker"` ;
- `longPathAware` = `true` ;
- `supportedOS` pour Windows 10 et 11.

L'icône et les informations de version sont aussi dans les ressources. L'outil `rc.exe` du SDK Windows est déjà présent sur la machine de développement.

### 10.3 Cibles

- `x86_64-pc-windows-msvc` : cible principale.
- `aarch64-pc-windows-msvc` : ajoutée en CI. Le surcoût est nul pour le code, car `windows-rs` gère les deux architectures. Elle ne sera pas testée manuellement faute de machine ARM.

### 10.4 Packaging (phase 6)

- Exécutable portable : zip avec les deux `.exe`. Le mode portable stocke la configuration à côté si un fichier `portable.txt` est présent.
- Installeur : **Inno Setup**. Il est simple, léger, installe par utilisateur sans droits d'administrateur, et son script est lisible. Alternative : MSI avec WiX (meilleur pour les déploiements en entreprise, plus lourd). Voir les questions ouvertes.
- CI GitHub Actions : build, `cargo clippy -- -D warnings`, `cargo fmt --check`, `cargo test`, vérification anti-injection, artefacts de release.

---

## 11. Garantie anti-injection vérifiable

Contrôles automatiques en CI et en local (`cargo xtask check-isolation` ou script) :
1. Grep qui échoue sur : `VirtualAllocEx`, `WriteProcessMemory`, `CreateRemoteThread`, `NtCreateThreadEx`, `QueueUserAPC`, `SetWindowsHookEx` avec un type autre que `WH_MOUSE_LL`, `DllRegisterServer`, `DllGetClassObject`.
2. Aucun crate du workspace n'a `crate-type = ["cdylib"]` ou `["dylib"]`. Le dossier de sortie ne contient aucun `.dll`.
3. Le crate `windows` n'active pas la fonctionnalité `Win32_System_Diagnostics_Debug` ni les autres modules de manipulation de processus.
4. L'installeur n'écrit aucune clé sous `HKCR\*\shellex`, `HKCR\Directory\Background\shellex` ni `HKCU\Software\Classes\CLSID`.

Vérification manuelle en fin de phases 1 et 5 : Process Explorer, recherche du handle ou de la DLL « bento ». Résultat attendu : seuls `bento-desk.exe` et éventuellement `bento-settings.exe` apparaissent.

---

## 12. Décisions validées (2026-11-25)

1. **Nom** : Bento Desk. Dossiers `%APPDATA%\BentoDesk` et `%LOCALAPPDATA%\BentoDesk\logs`.
2. **Cible de test** : Windows 10 22H2 uniquement, car l'app est à usage personnel sur cette machine. La compatibilité Windows 11 est visée dans le code, sans test. Les défauts Windows 11 seront corrigés plus tard.
3. **Paramètres** : Slint, avec attribution `AboutSlint` dans la section À propos.
4. **Démarrage** : clé `Run`. Le délai de quelques secondes après l'ouverture de session est accepté.
5. **Nombre d'ouvertures** : comptage des double-clics seulement. Pas d'UserAssist.
6. **Teinte d'icônes** : abandonnée. Les vraies icônes restent déplacées dans les fences, avec leurs couleurs d'origine.
7. **Geste clic droit + glisser** : livré dès la phase 1, avec le hook souris.
8. **Installeur** : Inno Setup, en plus de l'exécutable portable.
9. **Rust** : installation de `rustup-init.exe` depuis rustup.rs (toolchain stable, cible `x86_64-pc-windows-msvc`) au début de la phase 1. Les Build Tools Visual Studio 2019 (MSVC 14.29) et le SDK Windows 10.0.19041 déjà présents suffisent.

---

## 13. Risques

| # | Risque | Probabilité | Impact | Mitigation |
|---|---|---|---|---|
| R1 | Le message `0x052C` ou la structure de la couche bureau change dans une future version de Windows | Moyenne | Élevé | Détection par inspection de l'arbre des fenêtres et non par numéro de build ; journaux détaillés ; mode dégradé « fences au-dessus du bureau sans icônes » si l'insertion échoue. |
| R2 | Vanguard réagit au hook `WH_MOUSE_LL` | Faible | Élevé | Le hook n'existe que lorsque le bureau est au premier plan. Mode jeu. Option pour désactiver toutes les fonctions qui exigent le hook. Test réel en fin de phases 1 et 5. |
| R3 | Les clics ne traversent pas les pixels transparents d'une fenêtre enfant layered | Faible | Élevé | Spike S2. Repli : découper le `FenceFrame` en fenêtres séparées (titre, bords). |
| R4 | Conflit avec Wallpaper Engine ou Lively (même `WorkerW`) | Moyenne | Moyen | Réordonnancement dans l'ordre Z sur événement ; test avec Lively. |
| R5 | Explorer recadre les icônes parquées hors écran | Moyenne | Moyen | Spike S3. Repli : réappliquer après `TaskbarCreated` ; récupération manuelle dans Dépannage. |
| R6 | Le blocage du thread UI fige le bureau (files d'entrée attachées) | Faible | Élevé | Aucun appel bloquant sur le thread UI ; COM sur le thread Shell. |
| R7 | `SetCurrentFolderFlags(FWF_NOICONS)` se comporte de façon incohérente | Moyenne | Faible | Repli sur le parking. |
| R8 | Win+Espace non enregistrable | Élevée | Faible | Raccourci de repli et message clair. |
| R9 | Changement de DPI ou d'écran pendant un glisser | Faible | Faible | Annulation du glisser en cours et nouveau calcul. |

### Spikes du début de phase 1

Petits prototypes jetables, faits sur Windows 10 22H2 :

| Spike | Question |
|---|---|
| S1 | Un fond de fence s'affiche-t-il sous `SHELLDLL_DefView` avec la méthode prévue ? |
| S2 | Le `FenceFrame` au-dessus de `DefView` laisse-t-il passer les clics sur ses pixels transparents ? Les fences survivent-elles à Win+D ? |
| S3 | Les positions hors écran tiennent-elles, y compris après un redémarrage d'explorer ? Quel est le coût du redessin pour 50 icônes ? |
| S4 | `RegisterHotKey(MOD_WIN, VK_SPACE)` réussit-il ? |
| S5 | Le coût du hook LL quand le bureau est au premier plan est-il négligeable ? |
| S6 | Un clic droit reposté avec `PostMessage` affiche-t-il bien le menu natif du bureau ? |

---

## 14. Objectifs chiffrés au repos (daemon seul, 10 fences, 100 icônes)

| Mesure | Objectif | Outil |
|---|---|---|
| CPU au repos | 0,0 % en moyenne sur 10 minutes, aucun réveil périodique | Process Explorer (delta des context switches = 0 au repos), WPR et WPA |
| Minuteries actives au repos | 0 | Revue de code et traces |
| Mémoire privée | 15 Mo maximum | Process Explorer (Private Bytes) |
| Working set | 25 Mo maximum | Process Explorer |
| Handles | Stable sur 24 heures (pas de fuite) | Process Explorer, test longue durée |
| Objets GDI et USER | Stables sur 24 heures | Gestionnaire des tâches (colonnes GDI et USER) |
| Démarrage jusqu'aux fences affichées | 300 ms maximum après la disponibilité d'explorer | Journal horodaté |
| Taille de `bento-desk.exe` | 3 Mo maximum | Build release |
| Paramètres ouverts | 50 Mo maximum, 0 une fois fermés | Process Explorer |

La mesure est faite à la fin de chaque phase et comparée à ce tableau.

---

## 15. Découpage en phases (rappel)

1. **Phase 1** : spikes S1 à S6, fences de base (création par le menu et par clic droit + glisser, déplacement, redimensionnement, vraies icônes, persistance), tray, démarrage avec Windows, instance unique. Test anti-cheat demandé à la fin.
2. **Phase 2** : apparence, roll-up, quick-hide, magnétisme, défilement.
3. **Phase 3** : règles et tri, portails de dossiers.
4. **Phase 4** : layouts et snapshots, multi-écrans, DPI.
5. **Phase 5** : Peek, onglets, pages de bureau, Chameleon. Test anti-cheat demandé à la fin.
6. **Phase 6** : packaging, installeur, README, CI GitHub Actions.

À la fin de chaque phase : ce qui a été fait, comment le tester à la main, les limites connues, les mesures face aux objectifs, et un message de commit en une ligne.

---

## 16. Écarts constatés pendant la réalisation

### Surveillance des dossiers

`SHChangeNotifyRegister` ne livre aucun événement pour un Bureau redirigé par OneDrive, même avec des PIDL du système de fichiers et un enregistrement récursif. La surveillance utilise donc `ReadDirectoryChangesW` en mode synchrone, sur un thread par dossier. Le thread reste bloqué dans l'appel tant que rien ne change : aucun réveil périodique. Il est arrêté avec `CancelSynchronousIo`.

Doc : https://learn.microsoft.com/windows/win32/api/winbase/nf-winbase-readdirectorychangesw

Explorer ne rafraîchit pas toujours le bureau quand un fichier est créé par un autre outil. Après quelques secondes sans icône correspondante, Bento Desk appelle `SHChangeNotify(SHCNE_CREATE)`. Les icônes fantômes, dont le fichier n'existe plus, sont retirées avec `SHCNE_DELETE`. Ces appels sont des notifications documentées et ne chargent rien dans explorer.

### Raccourcis (spike S4)

Sur la machine de test, `RegisterHotKey` échoue pour Win+Espace, Ctrl+Win+Espace et Ctrl+Alt+Espace : ces combinaisons sont déjà prises. Ctrl+Alt+D et Ctrl+Win+Page suivante ou Page précédente s'enregistrent. Le raccourci Peek par défaut est donc **Ctrl+Alt+D**. Il se change dans la section Peek des paramètres, avec un enregistreur de combinaison.

### Fenêtre de paramètres

- Slint 1.19 avec le style Fluent, le backend winit et le **rendu logiciel**. Avec le rendu OpenGL (femtovg), le pilote graphique chargé porte la fenêtre à environ 150 Mo de working set. Le rendu logiciel la ramène à environ 22 Mo, pour un rendu identique dans une fenêtre de réglages.
- Le code généré par le compilateur Slint contient des blocs `unsafe`. Il est isolé dans le module `ui` du binaire, seul endroit où `unsafe_code` est autorisé. Le code écrit à la main dans `bento-settings` n'en contient pas.
- Traductions : textes source en anglais, français dans `apps/bento-settings/lang/fr/LC_MESSAGES/bento-settings.po`, intégré au binaire. Les textes calculés côté Rust (listes, résumés de règles, messages) sont dans `src/text.rs`.
- La fenêtre écrit `config.json` (avec un délai de 300 ms pendant les glissements de curseur) puis envoie `ReloadConfig`. Les changements propres à une fence passent par les commandes IPC, car `state.json` appartient à l'application résidente.
- Mica est demandé sur Windows 11 22H2 et plus. Windows 10 l'ignore et garde le fond uni du style Fluent.

