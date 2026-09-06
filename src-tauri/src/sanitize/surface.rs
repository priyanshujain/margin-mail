// Which surface a message reads on, and what has to come off it to read there.
//
// The rule is one sentence: a message renders on a light page when the sender painted one, and on
// the app's own surface when they did not. Almost nobody does this. Mailspring has a global
// three-state preference and, in its dark theme, `filter: invert(100%) hue-rotate(180deg)` over
// every body; it is currently retreating from that to always-light because the filters stack.
// Thunderbird sets a dark canvas and walks the DOM deleting author colours that would be
// unreadable on it, which is the right shape, but it applies that to every message including the
// ones whose design is a painted white page.
//
// Neither asks the question that actually decides it. A colleague's mail composed in a client that
// sends HTML carries bold runs, a list, some links and a signature, and no colour and no surface of
// its own. There is nothing in it that wants a white page, and putting it on one in a dark window
// is a slab of white in the middle of the app. A newsletter, by contrast, paints its own shell: a
// wash behind a 600 pixel card, a header band, a footer in grey. That one is a designed page and it
// should stay on the page it was designed for.
//
// So the predicate is about paint, not about `Content-Type`. `is_html` says how the bytes arrived;
// it says nothing about whether the sender laid out a page.
//
// The decision is made once, here, and stored on the body row, because opening a thread is a local
// read and is not allowed to grow a DOM walk.
//
// ---------------------------------------------------------------------------------------------
//
// One render is served to both palettes, which is the constraint that shapes everything below. The
// mirror caches one `html` per message and the theme can change under an open thread, so nothing
// here may bake in a light or a dark answer. What that costs is stated where it is paid: a colour
// is kept only when it reads on both of our surfaces, and a background is dropped in both palettes
// or in neither.

use super::{escape_html, scan, Tag};
use crate::dto::Surface;

/// The one bar every threshold below is derived from.
///
/// 3:1 rather than the 4.5:1 that WCAG asks of body text, and the reason is arithmetic rather than
/// laziness: no colour on earth clears 4.5:1 against a near-white page and a near-black one at the
/// same time, so a 4.5 here would read "drop every colour the sender chose". At 3:1 the surviving
/// band is the mid tones, which is where a brand red, a heading grey and a link blue live, and what
/// falls out of it is the near-black body text that would vanish on our dark page and the near-white
/// heading that would vanish on our light one. Those two are the whole problem.
const MIN_CONTRAST: f64 = 3.0;

/// Alpha below which a colour is a tint over something else rather than a surface of its own. A
/// translucent colour is defined against a backdrop, and the backdrop here is ours, not the
/// sender's, so anything under this is treated as not painted at all.
const MIN_COVERAGE: f64 = 0.9;

/// `--paper` from the light palette in margin-shared's token set, and its dark twin. They are here
/// as literals because the decision is made in Rust and cached, and threading a CSS custom property
/// through the render options would put the current theme into a cache shared by both of them.
/// A change to either token wants a change here; `the_thresholds_come_from_our_own_two_papers`
/// below is what says so out loud.
const LIGHT_PAPER: (u8, u8, u8) = (0xfc, 0xfb, 0xf7);
const DARK_PAPER: (u8, u8, u8) = (0x1d, 0x1a, 0x16);

/// The narrowest width at which a sender lays a whole message out. Below it a fixed width box is a
/// callout, a button or an image cell rather than the page.
const FULL_WIDTH_PX: f64 = 480.0;

/// How far down the spine to look. A newsletter nests tables inside tables inside tables and ten
/// deep is ordinary; past this it is content, not shell.
const SPINE_DEPTH: usize = 16;

/// Elements that can be the page. Everything inline is absent on purpose: a highlighted `<span>`
/// and a coloured `<font>` are marks on the text, not a surface under it, and so is a `<p>`.
const CONTAINERS: &[&str] = &[
    "body", "center", "div", "table", "tbody", "td", "tfoot", "th", "thead", "tr",
];

/// Elements with no closing tag, so the tree builder must not wait for one.
const VOID: &[&str] = &[
    "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "source", "track",
    "wbr",
];

// ---------------------------------------------------------------------------------------------
// The decision
// ---------------------------------------------------------------------------------------------

/// Whether the sender painted a page.
///
/// Both halves of the message are read, and they answer different halves of the question.
/// `sanitised` is what will actually be on screen, well formed because `html5ever` serialised it,
/// and it carries the two signals that survive the whitelist: `background-color` is in
/// `STYLE_PROPERTIES` and `bgcolor` is kept on the table family.
///
/// `source` is what the sender sent, and it is read for the paint that could not survive. Two
/// things go missing on the way through. `<body>` itself does, because the output is a fragment, so
/// a shell painted with `<body bgcolor>` leaves no trace in it at all. And `background`,
/// `background-image` and the shorthand do, because a URL in CSS is a fetch and a fetch is a
/// tracker, so a newsletter that paints its shell with an image rather than a colour is invisible
/// in the output.
///
/// Reading the source for that is safe and it is exact. Nothing from it is emitted: the whole
/// return value is one of two words. The alternative was the table shape, a full width table with a
/// fixed width one centred inside it, and that is a proxy for the thing rather than the thing. It
/// says yes to a plain reply somebody's client wrapped in a table, and it still says nothing about
/// whether anything was painted.
///
/// What is still missed is paint that only ever existed in a `<style>` rule, since a rule needs a
/// selector matched against a tree and this is a tag reader. Templates that inline their CSS, which
/// is most of them, are not affected.
pub fn decide(sanitised: &str, source: &str) -> Surface {
    let bounds = Bounds::new();
    let painted = spine_paints(sanitised, &bounds) || spine_paints(source, &bounds);
    if painted {
        Surface::Paper
    } else {
        Surface::Theme
    }
}

fn spine_paints(html: &str, bounds: &Bounds) -> bool {
    let tags = scan(html);
    let (nodes, roots) = tree(&tags);
    roots
        .iter()
        .any(|&root| paints_a_page(&nodes, &tags, root, 0, bounds))
}

fn paints_a_page(
    nodes: &[Element],
    tags: &[Tag],
    index: usize,
    depth: usize,
    bounds: &Bounds,
) -> bool {
    let node = &nodes[index];
    let tag = &tags[node.tag];

    if CONTAINERS.contains(&tag.name.as_str()) && paints(tag, bounds) {
        return true;
    }

    if depth >= SPINE_DEPTH {
        return false;
    }
    // A wrapper with one child is still the spine whatever it calls itself. With several, only a
    // child that says it is the full width is, which is what keeps a coloured signature block from
    // being read as the page.
    let sole = node.children.len() == 1;
    node.children.iter().any(|&child| {
        (sole || spans_the_layout(&tags[nodes[child].tag]))
            && paints_a_page(nodes, tags, child, depth + 1, bounds)
    })
}

/// Whether this one element covers itself in a light page.
fn paints(tag: &Tag, bounds: &Bounds) -> bool {
    // An image is a page whatever is in it. It cannot be measured, it is gone by the time the body
    // renders, and a sender who put one behind the whole shell laid out a page.
    if tag
        .attr("background")
        .is_some_and(|value| !value.trim().is_empty())
    {
        return true;
    }
    if let Some(style) = tag.attr("style") {
        for (property, value) in declarations(style) {
            let painted = match property {
                "background" | "background-image" => {
                    value.contains("url(") || is_a_page(shorthand_colour(value), bounds)
                }
                _ => false,
            };
            if painted {
                return true;
            }
        }
    }
    // A surface only counts when it covers and when it is light. An opaque dark shell is a sender
    // who designed for dark, and pinning them to a white page would be the same mistake in the
    // other direction; they take the theme branch, where the neutraliser leaves a dark background
    // alone and their design comes through intact.
    is_a_page(background_of(tag), bounds)
}

fn is_a_page(colour: Option<Rgba>, bounds: &Bounds) -> bool {
    colour.is_some_and(|colour| {
        colour.alpha >= MIN_COVERAGE && relative_luminance(colour) > bounds.page
    })
}

/// The colour out of a `background` shorthand, which is whichever of its parts is one.
fn shorthand_colour(value: &str) -> Option<Rgba> {
    value.split_whitespace().find_map(parse_colour)
}

fn spans_the_layout(tag: &Tag) -> bool {
    match tag.name.as_str() {
        // The document, which is only ever seen in the source: what the sanitiser returns is a
        // fragment with nothing around it.
        "html" | "body" => true,
        // The cells and the rows are missing from this deliberately. A `<td>` is reached through
        // the single child rule instead, which is the `<tr><td>` a newsletter shell is actually
        // built from, and a coloured cell in a four column table is not a page.
        "center" | "div" | "table" => match width_of(tag) {
            Some(Width::Percent(percent)) => percent >= 90.0,
            Some(Width::Pixels(pixels)) => pixels >= FULL_WIDTH_PX,
            None => false,
        },
        _ => false,
    }
}

// ---------------------------------------------------------------------------------------------
// The neutraliser
// ---------------------------------------------------------------------------------------------

/// Author colours that would be unreadable on our own paper, taken off so they inherit ours.
///
/// Thunderbird's shape, and the shape is the good part: for each element, no background under the
/// text means the text colour has to stand on its own, and a background under it means the pair is
/// judged together and the background is what gives way first. What is different here is the
/// thresholds, which are derived from our two papers rather than inherited, and the reach, which is
/// inline styles and the presentational colour attributes only.
///
/// A `<style>` block is not handled and there is nothing to handle: `ammonia` drops `style`
/// elements with their content, so an embedded stylesheet has already gone by the time this runs.
/// `mix-blend-mode` is the same story, absent from `STYLE_PROPERTIES` and so never in the output.
pub fn neutralise(html: &str) -> String {
    let tags = scan(html);
    let bounds = Bounds::new();
    let mut out = String::with_capacity(html.len());
    let mut cursor = 0usize;

    for tag in &tags {
        if tag.closing {
            continue;
        }
        let Some(rewritten) = without_unreadable_colours(tag, &bounds) else {
            continue;
        };
        out.push_str(&html[cursor..tag.start]);
        out.push_str(&rewritten);
        cursor = tag.end;
    }

    if cursor == 0 {
        return html.to_string();
    }
    out.push_str(&html[cursor..]);
    out
}

/// The tag written out again without the declarations that had to go, or `None` when none did.
fn without_unreadable_colours(tag: &Tag, bounds: &Bounds) -> Option<String> {
    let background = background_of(tag).filter(|colour| colour.alpha >= MIN_COVERAGE);
    let foreground = foreground_of(tag);
    let mut drop_background = false;
    let mut drop_foreground = false;

    match background {
        None => {
            drop_foreground = foreground.is_some_and(|colour| !bounds.legible(colour));
        }
        Some(background) => {
            let a_light_page = relative_luminance(background) > bounds.page;
            let sender_cannot_read_it =
                foreground.is_some_and(|colour| contrast(colour, background) < MIN_CONTRAST);
            if a_light_page || sender_cannot_read_it {
                drop_background = true;
                drop_foreground = foreground.is_some_and(|colour| !bounds.legible(colour));
            }
        }
    }

    if !drop_background && !drop_foreground {
        return None;
    }
    Some(rewrite(tag, drop_background, drop_foreground))
}

/// The open tag serialised again with the named colours gone.
///
/// Only tags that actually changed go through this; everything else is copied byte for byte, so a
/// `data:` image and a folded link come out of the neutraliser exactly as the sanitiser wrote them.
fn rewrite(tag: &Tag, drop_background: bool, drop_foreground: bool) -> String {
    let mut out = String::with_capacity(tag.end - tag.start);
    out.push('<');
    out.push_str(&tag.name);

    for (name, value) in &tag.attrs {
        let kept = match name.as_str() {
            "bgcolor" if drop_background => None,
            "color" if drop_foreground => None,
            "style" => {
                let style = declarations(value)
                    .filter(|(property, _)| match *property {
                        "background-color" => !drop_background,
                        "color" => !drop_foreground,
                        _ => true,
                    })
                    .map(|(property, value)| format!("{property}:{value}"))
                    .collect::<Vec<_>>()
                    .join(";");
                // An empty style attribute is not wrong, but it is litter in a golden file.
                if style.is_empty() {
                    None
                } else {
                    Some(style)
                }
            }
            _ => Some(value.clone()),
        };
        let Some(value) = kept else { continue };
        out.push(' ');
        out.push_str(name);
        out.push_str("=\"");
        out.push_str(&escape_html(&value));
        out.push('"');
    }

    out.push_str(if tag.self_closing { " />" } else { ">" });
    out
}

// ---------------------------------------------------------------------------------------------
// Reading colour off an element
// ---------------------------------------------------------------------------------------------

fn background_of(tag: &Tag) -> Option<Rgba> {
    declaration(tag, "background-color")
        .or_else(|| tag.attr("bgcolor").and_then(parse_colour))
}

/// The style attribute wins over the presentational one, which is what a browser does with them.
fn foreground_of(tag: &Tag) -> Option<Rgba> {
    declaration(tag, "color").or_else(|| tag.attr("color").and_then(parse_colour))
}

fn declaration(tag: &Tag, property: &str) -> Option<Rgba> {
    let style = tag.attr("style")?;
    // From the back, because a property declared twice is whichever one came last.
    declarations(style)
        .rfind(|(name, _)| *name == property)
        .and_then(|(_, value)| parse_colour(value))
}

/// The declarations of a style attribute, in order.
///
/// A split rather than a parser. On the sanitiser's own output that is exact: `ammonia` has already
/// run the attribute through `cssparser` and written it back as `name:value;name:value` with the
/// property names lowercased by the whitelist lookup and every value a sequence of complete tokens,
/// so no value there can hold a semicolon.
///
/// On the source it is a reading rather than a parse, and it is allowed to be, because the only
/// thing read from the source is whether a shell was painted. A sender who hides a semicolon inside
/// a quoted font stack gets a declaration split in two, and the answer that falls out of that is
/// the app's own surface, which is the end of this nothing goes wrong at.
fn declarations(style: &str) -> impl DoubleEndedIterator<Item = (&str, &str)> {
    style.split(';').filter_map(|declaration| {
        let (name, value) = declaration.split_once(':')?;
        Some((name.trim(), value.trim()))
    })
}

enum Width {
    Percent(f64),
    Pixels(f64),
}

/// What the element says about its own width, from the attribute or from the style.
fn width_of(tag: &Tag) -> Option<Width> {
    let from_style = tag.attr("style").and_then(|style| {
        declarations(style)
            .find(|(name, _)| *name == "width" || *name == "max-width")
            .and_then(|(_, value)| parse_width(value))
    });
    from_style.or_else(|| tag.attr("width").and_then(parse_width))
}

fn parse_width(value: &str) -> Option<Width> {
    let value = value.trim();
    if let Some(percent) = value.strip_suffix('%') {
        return percent.trim().parse::<f64>().ok().map(Width::Percent);
    }
    let digits: String = value
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect();
    digits.parse::<f64>().ok().map(Width::Pixels)
}

// ---------------------------------------------------------------------------------------------
// Colour
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq)]
struct Rgba {
    red: f64,
    green: f64,
    blue: f64,
    alpha: f64,
}

/// The names a mail client actually writes, plus the sixteen HTML 4 keywords in full.
///
/// Not all hundred and forty: an unrecognised name reads as "no colour here", which leaves the
/// element alone, and leaving something alone is the end of this you want to fall off.
const NAMED: &[(&str, u32)] = &[
    ("aqua", 0x00ffff),
    ("azure", 0xf0ffff),
    ("beige", 0xf5f5dc),
    ("black", 0x000000),
    ("blue", 0x0000ff),
    ("brown", 0xa52a2a),
    ("cyan", 0x00ffff),
    ("darkblue", 0x00008b),
    ("darkgray", 0xa9a9a9),
    ("darkgrey", 0xa9a9a9),
    ("dimgray", 0x696969),
    ("dimgrey", 0x696969),
    ("fuchsia", 0xff00ff),
    ("gainsboro", 0xdcdcdc),
    ("gold", 0xffd700),
    ("gray", 0x808080),
    ("green", 0x008000),
    ("grey", 0x808080),
    ("ivory", 0xfffff0),
    ("lightgray", 0xd3d3d3),
    ("lightgrey", 0xd3d3d3),
    ("lime", 0x00ff00),
    ("linen", 0xfaf0e6),
    ("magenta", 0xff00ff),
    ("maroon", 0x800000),
    ("navy", 0x000080),
    ("olive", 0x808000),
    ("orange", 0xffa500),
    ("pink", 0xffc0cb),
    ("purple", 0x800080),
    ("red", 0xff0000),
    ("silver", 0xc0c0c0),
    ("snow", 0xfffafa),
    ("teal", 0x008080),
    ("white", 0xffffff),
    ("whitesmoke", 0xf5f5f5),
    ("yellow", 0xffff00),
];

fn parse_colour(value: &str) -> Option<Rgba> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    let lower = value.to_ascii_lowercase();
    if lower == "transparent" {
        return Some(Rgba { red: 0.0, green: 0.0, blue: 0.0, alpha: 0.0 });
    }
    if let Some(hex) = lower.strip_prefix('#') {
        return from_hex(hex);
    }
    if lower.starts_with("rgb") {
        return from_rgb_function(&lower);
    }
    if let Some((_, packed)) = NAMED.iter().find(|(name, _)| *name == lower) {
        return Some(from_packed(*packed));
    }
    // A `bgcolor` written the way 1997 wrote it, with the hash left off.
    from_hex(&lower)
}

fn from_packed(packed: u32) -> Rgba {
    Rgba {
        red: ((packed >> 16) & 0xff) as f64,
        green: ((packed >> 8) & 0xff) as f64,
        blue: (packed & 0xff) as f64,
        alpha: 1.0,
    }
}

fn from_hex(hex: &str) -> Option<Rgba> {
    if !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let pair = |at: usize| u8::from_str_radix(&hex[at..at + 2], 16).ok().map(f64::from);
    let single = |at: usize| {
        u8::from_str_radix(&hex[at..at + 1], 16)
            .ok()
            .map(|value| f64::from(value * 17))
    };
    match hex.len() {
        3 | 4 => Some(Rgba {
            red: single(0)?,
            green: single(1)?,
            blue: single(2)?,
            alpha: if hex.len() == 4 { single(3)? / 255.0 } else { 1.0 },
        }),
        6 | 8 => Some(Rgba {
            red: pair(0)?,
            green: pair(2)?,
            blue: pair(4)?,
            alpha: if hex.len() == 8 { pair(6)? / 255.0 } else { 1.0 },
        }),
        _ => None,
    }
}

/// `rgb(1, 2, 3)`, `rgba(1, 2, 3, 0.5)` and the modern `rgb(1 2 3 / 50%)`, which are the three
/// forms that reach here once `cssparser` has written the attribute back out.
fn from_rgb_function(value: &str) -> Option<Rgba> {
    let inside = value.split_once('(')?.1.strip_suffix(')')?;
    let parts: Vec<&str> = inside
        .split([',', '/', ' '])
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .collect();
    if parts.len() < 3 {
        return None;
    }
    let channel = |part: &str| -> Option<f64> {
        match part.strip_suffix('%') {
            Some(percent) => percent.parse::<f64>().ok().map(|p| p * 255.0 / 100.0),
            None => part.parse::<f64>().ok(),
        }
    };
    let alpha = match parts.get(3) {
        None => 1.0,
        Some(part) => match part.strip_suffix('%') {
            Some(percent) => percent.parse::<f64>().ok()? / 100.0,
            None => part.parse::<f64>().ok()?,
        },
    };
    Some(Rgba {
        red: channel(parts[0])?,
        green: channel(parts[1])?,
        blue: channel(parts[2])?,
        alpha,
    })
}

// ---------------------------------------------------------------------------------------------
// The two numbers everything is decided against
// ---------------------------------------------------------------------------------------------

/// The luminance band an author colour has to land in, worked out from our own two papers.
///
/// `page` is where a colour stops contrasting with our light paper at `MIN_CONTRAST`, and `ink` is
/// where it starts contrasting with our dark one. Above `page` a colour is a page rather than ink,
/// which is why the same number decides both "this text would vanish on our light theme" and "this
/// background is a light slab in our dark one".
///
/// This is real WCAG relative luminance, linearised, rather than Thunderbird's
/// `0.2125r + 0.7154g + 0.0721b` over raw sRGB bytes with a threshold of 200. Their number cannot
/// be reasoned about: it is neither a luminance anybody else computes nor a contrast ratio, and
/// their `CONTRAST_THRESHOLD = 3.5` reuses WCAG's `+0.05` constants on a 0..255 scale, so it is not
/// a 3.5:1 ratio either. These two are one bar, applied twice, and the arithmetic is above.
struct Bounds {
    page: f64,
    ink: f64,
}

impl Bounds {
    fn new() -> Self {
        let light = relative_luminance(from_packed(pack(LIGHT_PAPER)));
        let dark = relative_luminance(from_packed(pack(DARK_PAPER)));
        Bounds {
            page: (light + 0.05) / MIN_CONTRAST - 0.05,
            ink: MIN_CONTRAST * (dark + 0.05) - 0.05,
        }
    }

    /// Readable on both of our papers, which is what one cached render for two palettes means.
    fn legible(&self, colour: Rgba) -> bool {
        if colour.alpha < MIN_COVERAGE {
            return false;
        }
        let luminance = relative_luminance(colour);
        luminance >= self.ink && luminance <= self.page
    }
}

fn contrast(one: Rgba, other: Rgba) -> f64 {
    let a = relative_luminance(one);
    let b = relative_luminance(other);
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}

fn pack((red, green, blue): (u8, u8, u8)) -> u32 {
    (u32::from(red) << 16) | (u32::from(green) << 8) | u32::from(blue)
}

fn relative_luminance(colour: Rgba) -> f64 {
    let channel = |value: f64| {
        let value = (value / 255.0).clamp(0.0, 1.0);
        if value <= 0.03928 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * channel(colour.red) + 0.7152 * channel(colour.green) + 0.0722 * channel(colour.blue)
}

// ---------------------------------------------------------------------------------------------
// The tree
// ---------------------------------------------------------------------------------------------

/// One element and the elements under it. Text is not in here: nothing this module decides depends
/// on what a node says, only on what it paints.
struct Element {
    tag: usize,
    children: Vec<usize>,
}

/// Parent and child, from the flat tag list.
///
/// The input is `ammonia`'s output, which `html5ever` serialised, so every non-void element carries
/// its closing tag and the nesting is already well formed. A stray close is ignored rather than
/// unwinding the stack, which is the safe end: a misread tree can only ever fail to find a painted
/// page, and failing to find one puts the message on the theme where the app's own colours apply.
fn tree(tags: &[Tag]) -> (Vec<Element>, Vec<usize>) {
    let mut nodes: Vec<Element> = Vec::new();
    let mut roots: Vec<usize> = Vec::new();
    let mut open: Vec<usize> = Vec::new();

    for (index, tag) in tags.iter().enumerate() {
        if tag.closing {
            if let Some(at) = open
                .iter()
                .rposition(|&node| tags[nodes[node].tag].name == tag.name)
            {
                open.truncate(at);
            }
            continue;
        }
        let node = nodes.len();
        nodes.push(Element { tag: index, children: Vec::new() });
        match open.last() {
            Some(&parent) => nodes[parent].children.push(node),
            None => roots.push(node),
        }
        if !tag.self_closing && !VOID.contains(&tag.name.as_str()) {
            open.push(node);
        }
    }

    (nodes, roots)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn luminance_of(css: &str) -> f64 {
        relative_luminance(parse_colour(css).expect(css))
    }

    // -------------------------------------------------------------------------------------
    // The thresholds
    // -------------------------------------------------------------------------------------

    #[test]
    fn the_thresholds_come_from_our_own_two_papers() {
        let bounds = Bounds::new();
        // Both bars are the same 3:1, one measured against each of our papers. If a palette token
        // moves, the literals at the top of this file have to move with it and this is the test
        // that fails when they have not.
        let light = luminance_of("#fcfbf7");
        let dark = luminance_of("#1d1a16");
        assert!(((light + 0.05) / (bounds.page + 0.05) - MIN_CONTRAST).abs() < 1e-9);
        assert!(((bounds.ink + 0.05) / (dark + 0.05) - MIN_CONTRAST).abs() < 1e-9);
    }

    #[test]
    fn a_colour_survives_only_when_it_reads_on_both_of_our_papers() {
        let bounds = Bounds::new();
        let legible = |css: &str| bounds.legible(parse_colour(css).expect(css));

        // The band that survives: a brand red, a mid grey, a link blue.
        assert!(legible("#d32f2f"));
        assert!(legible("#666666"));
        assert!(legible("#0a66c2"));

        // Too dark to read on our dark paper.
        assert!(!legible("#000000"));
        assert!(!legible("#333333"));
        assert!(!legible("black"));

        // Too light to read on our light one, which is the half nobody handles: a sender who wrote
        // white text for their own dark design leaves it invisible on a white page.
        assert!(!legible("#ffffff"));
        assert!(!legible("white"));
        assert!(!legible("#ece6da"));

        // A tint is defined against a backdrop we have already replaced.
        assert!(!legible("rgba(0, 0, 0, 0.87)"));
    }

    // -------------------------------------------------------------------------------------
    // Colour parsing
    // -------------------------------------------------------------------------------------

    #[test]
    fn colours_arrive_in_every_form_a_sender_writes_them_in() {
        let same = |a: &str, b: &str| {
            assert_eq!(parse_colour(a), parse_colour(b), "{a} and {b}");
        };
        same("#fff", "#ffffff");
        same("#FFF", "white");
        same("rgb(255, 255, 255)", "#ffffff");
        same("rgb(255 255 255)", "#ffffff");
        // A `bgcolor` written without its hash, which is how a 2003 newsletter still writes it.
        same("ffffff", "#ffffff");

        assert_eq!(parse_colour("transparent").expect("transparent").alpha, 0.0);
        assert_eq!(parse_colour("rgba(0, 0, 0, 0.5)").expect("rgba").alpha, 0.5);
        assert_eq!(parse_colour("rgb(0 0 0 / 50%)").expect("slash").alpha, 0.5);
        assert_eq!(parse_colour("currentColor"), None);
        assert_eq!(parse_colour("inherit"), None);
    }

    // -------------------------------------------------------------------------------------
    // The decision
    // -------------------------------------------------------------------------------------

    #[test]
    fn a_message_that_paints_nothing_takes_the_theme() {
        // The colleague's mail: bold, a list, links, a signature, and not one colour.
        let html = "<div><p>Morning,</p><p>Three things <b>before</b> Thursday:</p>\
                    <ul><li>the <b>lease</b></li><li>the invoice</li></ul>\
                    <p><a href=\"https://example.org/a\">the draft</a></p>\
                    <p>Arun<br>Meridian Properties</p></div>";
        assert_eq!(decide(html, html), Surface::Theme);
    }

    #[test]
    fn a_wrapper_with_a_background_colour_is_a_painted_page() {
        let html = "<div style=\"background-color:#f7f7f7;padding:24px\"><p>Hello</p></div>";
        assert_eq!(decide(html, html), Surface::Paper);
    }

    #[test]
    fn a_table_shell_with_bgcolor_is_a_painted_page() {
        let html = "<table width=\"100%\" bgcolor=\"#f4f4f4\"><tr><td>\
                    <table width=\"600\" bgcolor=\"#ffffff\"><tr><td>Sale</td></tr></table>\
                    </td></tr></table>";
        assert_eq!(decide(html, html), Surface::Paper);
    }

    #[test]
    fn a_full_width_table_with_no_colour_on_it_is_not_a_page() {
        // The width alone is a layout, not a design. Half the replies in a corpus are wrapped in
        // one of these and none of them wants a white slab.
        let html = "<table width=\"100%\"><tr><td>Just the numbers</td></tr></table>";
        assert_eq!(decide(html, html), Surface::Theme);
    }

    #[test]
    fn a_dark_shell_is_the_senders_own_dark_design_rather_than_a_page() {
        let html = "<div style=\"background-color:#111111;color:#eeeeee\"><p>Tonight</p></div>";
        assert_eq!(decide(html, html), Surface::Theme);
    }

    #[test]
    fn a_highlight_or_a_coloured_signature_is_not_the_page() {
        let highlight = "<div><p>See <span style=\"background-color:#ffff00\">this</span></p>\
                         <p>and this</p></div>";
        assert_eq!(decide(highlight, highlight), Surface::Theme);

        // Four siblings, one of them a signature block with a wash behind it. It is not the only
        // child and it does not claim the full width, so the spine stops above it.
        let signature = "<div><p>One</p><p>Two</p><p>Three</p>\
                         <div style=\"background-color:#eeeeee\">Arun</div></div>";
        assert_eq!(decide(signature, signature), Surface::Theme);
    }

    #[test]
    fn a_shell_painted_with_an_image_is_still_a_painted_page() {
        // The gap the whitelist leaves: `background-image` is a URL and a URL is a fetch, so the
        // sanitiser takes it out and the rendered body carries no colour at all. The source still
        // says what the sender did.
        let source = "<html><body><table width=\"100%\"                       style=\"background-image:url(https://cdn.example/shell.png)\">                      <tr><td>Sale</td></tr></table></body></html>";
        let sanitised = "<table width=\"100%\"><tr><td>Sale</td></tr></table>";
        assert_eq!(decide(sanitised, source), Surface::Paper);
        // And the width on its own, with the paint taken away, is not a page.
        assert_eq!(decide(sanitised, sanitised), Surface::Theme);
    }

    #[test]
    fn a_shell_painted_on_the_body_element_survives_it_being_thrown_away() {
        // `<body>` is not in the sanitiser's output at all, so a shell painted there leaves no
        // trace in the fragment and the source is the only place left to read it.
        let source = "<html><body bgcolor=\"#f4f4f4\"><p>Sale</p></body></html>";
        assert_eq!(decide("<p>Sale</p>", source), Surface::Paper);
    }

    #[test]
    fn a_transparent_background_paints_nothing() {
        let html = "<div style=\"background-color:transparent\"><p>Hello</p></div>";
        assert_eq!(decide(html, html), Surface::Theme);
        let tinted = "<div style=\"background-color:rgba(255, 255, 255, 0.4)\"><p>Hi</p></div>";
        assert_eq!(decide(tinted, tinted), Surface::Theme);
    }

    // -------------------------------------------------------------------------------------
    // The neutraliser
    // -------------------------------------------------------------------------------------

    #[test]
    fn a_light_author_colour_is_left_alone_and_a_dark_one_is_taken_off() {
        let kept = "<p style=\"color:#d32f2f\">Overdue</p>";
        assert_eq!(neutralise(kept), kept);

        let dropped = "<p style=\"color:#333333\">Morning</p>";
        assert_eq!(neutralise(dropped), "<p>Morning</p>");
    }

    #[test]
    fn a_colour_beside_other_declarations_takes_only_itself_with_it() {
        let html = "<p style=\"margin:0;color:#111111;font-weight:700\">Morning</p>";
        assert_eq!(neutralise(html), "<p style=\"margin:0;font-weight:700\">Morning</p>");
    }

    #[test]
    fn a_white_slab_goes_and_the_text_on_it_goes_with_it() {
        let html = "<td bgcolor=\"#ffffff\" style=\"color:#222222;padding:8px\">Total</td>";
        assert_eq!(neutralise(html), "<td style=\"padding:8px\">Total</td>");
    }

    #[test]
    fn a_dark_panel_the_sender_designed_survives_whole() {
        let html = "<div style=\"background-color:#111111;color:#eeeeee\">Tonight</div>";
        assert_eq!(neutralise(html), html);
    }

    #[test]
    fn a_button_keeps_its_colours_because_the_text_on_it_reads() {
        let html = "<td bgcolor=\"#0a66c2\" style=\"color:#ffffff\">Read more</td>";
        assert_eq!(neutralise(html), html);
    }

    #[test]
    fn a_font_elements_colour_attribute_is_a_colour_too() {
        let html = "<font color=\"#000000\">Terms</font>";
        assert_eq!(neutralise(html), "<font>Terms</font>");
        let kept = "<font color=\"#d32f2f\">Terms</font>";
        assert_eq!(neutralise(kept), kept);
    }

    #[test]
    fn everything_the_neutraliser_did_not_touch_comes_out_byte_for_byte() {
        let html = "<div><img src=\"data:image/png;base64,iVBORw0KGgo=\" alt=\"A &amp; B\">\
                    <a href=\"https://example.org/?a=1&amp;b=2\" target=\"_blank\">Link</a>\
                    <p style=\"color:#000000\">Ink</p></div>";
        let out = neutralise(html);
        assert!(out.contains("data:image/png;base64,iVBORw0KGgo="));
        assert!(out.contains("href=\"https://example.org/?a=1&amp;b=2\""));
        assert!(out.contains("alt=\"A &amp; B\""));
        assert!(out.contains("<p>Ink</p>"));
    }

    #[test]
    fn a_body_with_nothing_to_take_off_is_returned_unchanged() {
        let html = "<p>Morning</p><ul><li>One</li></ul>";
        assert_eq!(neutralise(html), html);
    }
}
