pub fn hsl_string(h: f32, s: f32, l: f32) -> String {
    format!("hsl({:.0}, {:.0}%, {:.0}%)", h, s * 100.0, l * 100.0)
}

pub fn hex_to_hsl(hex: &str) -> Result<(f32, f32, f32), ()> {
    let (r, g, b) = hex_to_rgb(hex)?;
    let r = r as f32 / 255.0;
    let g = g as f32 / 255.0;
    let b = b as f32 / 255.0;
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let l = (max + min) / 2.0;
    let d = max - min;
    let (h, s);
    if d == 0.0 {
        h = 0.0;
        s = 0.0;
    } else {
        s = if l > 0.5 { d / (2.0 - max - min) } else { d / (max + min) };
        h = if max == r {
            (g - b) / d + if g < b { 6.0 } else { 0.0 }
        } else if max == g {
            (b - r) / d + 2.0
        } else {
            (r - g) / d + 4.0
        } / 6.0;
    }
    Ok((h * 360.0, s, l))
}

pub fn hex_to_rgb(hex: &str) -> Result<(u8, u8, u8), ()> {
    let hex = hex.trim_start_matches('#');
    if hex.len() == 6 {
        if let Ok(r) = u8::from_str_radix(&hex[0..2], 16) {
            if let Ok(g) = u8::from_str_radix(&hex[2..4], 16) {
                if let Ok(b) = u8::from_str_radix(&hex[4..6], 16) {
                    return Ok((r, g, b));
                }
            }
        }
    }
    Err(())
}

pub fn derive_color_shades_with_bg(primary: &str, bg_color: &str, transition_hue: bool) -> Vec<String> {
    if let (Ok((h1, s1, l1)), Ok((h2, s2, l2))) = (hex_to_hsl(bg_color), hex_to_hsl(primary)) {
        let steps = 5;
        (0..steps)
            .map(|i| {
                let t = i as f32 / (steps - 1) as f32;
                let h = if transition_hue {
                    h1 + (h2 - h1) * t
                } else {
                    if i == 0 { h1 } else { h2 }
                };
                let s = s1 + (s2 - s1) * t;
                let l = l1 + (l2 - l1) * t;
                hsl_string(h, s, l)
            })
            .collect()
    } else {
        vec![
            bg_color.to_string(),
            "hsl(0, 0%, 70%)".to_string(),
            "hsl(0, 0%, 50%)".to_string(),
            "hsl(0, 0%, 35%)".to_string(),
            "hsl(0, 0%, 20%)".to_string(),
        ]
    }
} 
