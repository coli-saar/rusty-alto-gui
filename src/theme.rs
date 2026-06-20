use iced::{
    Border, Color, Shadow, Theme,
    widget::{button, container},
};

pub const SIDEBAR_WIDTH: f32 = 280.0;
pub const PAGE_PADDING: f32 = 18.0;
pub const SECTION_SPACING: f32 = 12.0;
pub const TABLE_ROW_HEIGHT: f32 = 28.0;
pub const TAB_HEIGHT: f32 = 38.0;

pub const BG: Color = Color::from_rgb(0.965, 0.971, 0.980);
pub const CANVAS: Color = Color::from_rgb(0.985, 0.988, 0.994);
pub const SIDEBAR: Color = Color::from_rgb(0.925, 0.941, 0.961);
pub const SURFACE: Color = Color::WHITE;
pub const SURFACE_RAISED: Color = Color::from_rgb(0.975, 0.980, 0.989);
pub const HOVER: Color = Color::from_rgb(0.875, 0.910, 0.955);
pub const BORDER: Color = Color::from_rgb(0.745, 0.790, 0.845);
pub const TEXT: Color = Color::from_rgb(0.125, 0.155, 0.200);
pub const MUTED: Color = Color::from_rgb(0.390, 0.440, 0.505);
pub const ACCENT: Color = Color::from_rgb(0.145, 0.420, 0.735);
pub const ACCENT_SOFT: Color = Color::from_rgb(0.830, 0.900, 0.980);
pub const SUCCESS: Color = Color::from_rgb(0.180, 0.600, 0.365);
pub const DANGER: Color = Color::from_rgb(0.790, 0.220, 0.250);

pub fn app_theme(_: &crate::app::Workbench) -> Theme {
    Theme::Light
}

pub fn panel(_: &Theme) -> container::Style {
    container::Style {
        background: Some(SURFACE.into()),
        border: Border {
            color: BORDER,
            width: 1.0,
            radius: 5.0.into(),
        },
        text_color: Some(TEXT),
        shadow: Shadow::default(),
    }
}

pub fn raised(_: &Theme) -> container::Style {
    container::Style {
        background: Some(SURFACE.into()),
        border: Border {
            color: BORDER,
            width: 1.0,
            radius: 6.0.into(),
        },
        text_color: Some(TEXT),
        shadow: Shadow::default(),
    }
}

pub fn flat(_: &Theme) -> container::Style {
    container::Style {
        background: Some(BG.into()),
        text_color: Some(TEXT),
        ..container::Style::default()
    }
}

pub fn sidebar(_: &Theme) -> container::Style {
    container::Style {
        background: Some(SIDEBAR.into()),
        text_color: Some(TEXT),
        ..container::Style::default()
    }
}

pub fn workspace(_: &Theme) -> container::Style {
    container::Style {
        background: Some(CANVAS.into()),
        text_color: Some(TEXT),
        ..container::Style::default()
    }
}

pub fn tab_strip(_: &Theme) -> container::Style {
    container::Style {
        background: Some(BG.into()),
        border: Border {
            color: BORDER,
            width: 0.0,
            radius: 0.0.into(),
        },
        text_color: Some(TEXT),
        ..container::Style::default()
    }
}

pub fn selected_button(theme: &Theme, status: button::Status) -> button::Style {
    let mut style = button::secondary(theme, status);
    style.background = Some(
        match status {
            button::Status::Hovered => HOVER,
            _ => ACCENT_SOFT,
        }
        .into(),
    );
    style.text_color = TEXT;
    style.border = Border {
        color: ACCENT,
        width: 1.0,
        radius: 5.0.into(),
    };
    style
}

pub fn quiet_button(theme: &Theme, status: button::Status) -> button::Style {
    let mut style = button::text(theme, status);
    style.text_color = match status {
        button::Status::Disabled => MUTED,
        _ => TEXT,
    };
    if matches!(status, button::Status::Hovered | button::Status::Pressed) {
        style.background = Some(HOVER.into());
    }
    style.border.radius = 5.0.into();
    style
}

pub fn parse_button(theme: &Theme, status: button::Status) -> button::Style {
    let mut style = button::primary(theme, status);
    style.background = Some(
        match status {
            button::Status::Hovered => Color::from_rgb(0.105, 0.355, 0.650),
            button::Status::Disabled => BORDER,
            _ => ACCENT,
        }
        .into(),
    );
    style.text_color = Color::WHITE;
    style.border.radius = 6.0.into();
    style
}
