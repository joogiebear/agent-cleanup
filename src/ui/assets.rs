//! Icons and fonts compiled into the exe.
//!
//! gpui-kit's default asset source only carries the icons its own widgets
//! use. The extra Lucide icons the app draws are listed in `ExtraIcons`; add a
//! name there (Lucide's file name in PascalCase) to use another one.

use gpui_kit::{AssetSource, Result, SharedString};
use std::borrow::Cow;

gpui_kit::assets::icon_assets!(
    ExtraIcons,
    [
        BroomSparkles,
        Broom,
        FolderHeart,
        FolderGit2,
        FolderSearch,
        FolderPlus,
        FolderMinus,
        Logs,
        Recycle,
        Clock,
        GitBranch,
        ShieldCheck,
        Download,
        Sparkles,
        Trash,
        Terminal,
        Hourglass,
        LifeBuoy,
        CircleQuestionMark,
        ArchiveRestore,
        ListChecks,
        Scan,
        Folders,
        Footprints,
        FilePen,
        RefreshCw,
        Package,
        CircleArrowUp,
        SquareTerminal,
    ]
);

pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        if let Some(bytes) = ExtraIcons.load(path)? {
            return Ok(Some(bytes));
        }
        gpui_kit::assets::Assets.load(path)
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut paths = gpui_kit::assets::Assets.list(path)?;
        paths.extend(ExtraIcons.list(path)?);
        paths.sort();
        paths.dedup();
        Ok(paths)
    }
}

pub const FONTS: [&[u8]; 3] = [
    include_bytes!("../../assets/fonts/Inter-Regular.ttf"),
    include_bytes!("../../assets/fonts/Inter-Medium.ttf"),
    include_bytes!("../../assets/fonts/Inter-SemiBold.ttf"),
];
