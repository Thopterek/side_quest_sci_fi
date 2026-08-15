//! Interactive investigator sheet - 3 pages, all fields fillable in any
//! reader that supports AcroForms (Acrobat, Firefox, Okular, Chrome...).
//!
//! Layout is loosely modelled on the classic 1920s / pulp investigator
//! sheets: characteristics with regular / half / fifth boxes, a skill list
//! with tick boxes for improvement rolls, weapons table, backstory blocks.
//!
//! Everything below is template text - swap the strings, the skill list and
//! the block titles for whatever your table actually uses.

use pdf_writer::types::{
    ActionType, AnnotationFlags, CheckBoxState, FieldFlags, FieldType, Quadding,
};
use pdf_writer::{Content, Filter, Finish, Name, Pdf, Rect, Ref, Str, TextStr};

/* ------------------------------------------------------------------ *
 * Theme + geometry. One place to retune the whole sheet.
 * PDF origin is the BOTTOM-left corner, so bigger y = further up.
 * ------------------------------------------------------------------ */

const PAGE_W: f32 = 595.0; // A4
const PAGE_H: f32 = 842.0;
const MARGIN: f32 = 30.0;
const RIGHT: f32 = PAGE_W - MARGIN; // 565.0
const FULL_W: f32 = RIGHT - MARGIN; // 535.0

// Resource names for the fonts. These must match the names we put into the
// page /Resources and into the AcroForm /DR, because a field's appearance
// string ("/Helv 9 Tf 0 g") looks the font up by name. Helv / HeBo /
// ZaDb are the conventional names that other form tools expect to find.
const F_TEXT: Name<'static> = Name(b"Helv");
const F_BOLD: Name<'static> = Name(b"HeBo");
const F_SYMBOL: Name<'static> = Name(b"ZaDb");

const INK: f32 = 0.10; // body text
const CAPTION_INK: f32 = 0.40; // the little labels above boxes
const RULE: f32 = 0.62; // box outlines
const TINT: f32 = 0.945; // box fill
const BANNER: f32 = 0.15; // section header bars

const BOX_H: f32 = 16.0; // default height of a one-line input
const CAP_DY: f32 = 3.6; // caption baseline sits this far above a box
const CHECK: f32 = 11.0; // checkbox / radio side length

const PAGES: usize = 2;

// Reserved artwork squares in the four corners. The title plate and the
// footer rule shrink to sit between them, so nothing else had to move.
const CORNER_W: f32 = 76.0;
const CORNER_H: f32 = 36.0;
const CORNER_X: f32 = 22.0; // inset from the paper edge, print-safe
const CORNER_TOP_Y: f32 = 792.0;
const CORNER_BOTTOM_Y: f32 = 24.0;
/// Horizontal span left over for the title plate and the footer.
const PLATE_X: f32 = CORNER_X + CORNER_W + 8.0; // 106.0
const PLATE_W: f32 = PAGE_W - 2.0 * PLATE_X; // 383.0
const PLATE_R: f32 = PLATE_X + PLATE_W; // 489.0

/* ------------------------------------------------------------------ *
 * Small config struct so a field can be described in one line.
 * ------------------------------------------------------------------ */

#[derive(Clone, Copy)]
struct Input {
    size: f32,
    quad: Quadding,
    // Stored as raw bits: pdf-writer's FieldFlags is a bitflags type that
    // does not derive Copy, and we want Input to stay Copy.
    flags: u32,
    tinted: bool,
}

impl Input {
    fn new() -> Self {
        Self {
            size: 9.0,
            quad: Quadding::Left,
            // Spell check squiggles under "Cthulhu" help nobody.
            flags: FieldFlags::DO_NOT_SPELL_CHECK.bits(),
            tinted: true,
        }
    }
    fn size(mut self, size: f32) -> Self {
        self.size = size;
        self
    }
    fn center(mut self) -> Self {
        self.quad = Quadding::Center;
        self
    }
    fn multiline(mut self) -> Self {
        self.flags |= FieldFlags::MULTILINE.bits();
        self
    }
    /// No fill / outline underneath - used inside ruled tables.
    fn bare(mut self) -> Self {
        self.tinted = false;
        self
    }
}

/// Vector art stored once in the file and painted wherever you like.
#[derive(Clone, Copy)]
struct Art {
    name: Name<'static>,
    w: f32,
    h: f32,
}

/// What a push button does when clicked.
enum ButtonAction {
    /// A named action: PrevPage / NextPage are core PDF and work everywhere.
    /// SaveAs, GoBack, GoForward and Print are Acrobat additions - readers
    /// that do not know them simply ignore the click.
    Named(&'static [u8]),
    /// ResetForm limited to an include list: only the listed fields are
    /// cleared. Unused by the current footer, kept for per-section clears.
    #[allow(dead_code)]
    ResetFields(Vec<Ref>),
}

/* ------------------------------------------------------------------ *
 * The builder. It owns the Pdf, the content stream of the page being
 * drawn, and the bookkeeping lists (widgets per page, root fields for
 * the AcroForm).
 * ------------------------------------------------------------------ */

struct Sheet {
    pdf: Pdf,
    next_id: i32,
    canvas: Content,       // static artwork of the current page
    annots: Vec<Ref>,      // widgets sitting on the current page
    root_fields: Vec<Ref>, // every top level field, for /AcroForm /Fields
    pages: Vec<Ref>,
    art: Vec<(Name<'static>, Ref)>, // form xobjects shared by every page
    page_tree: Ref,
    font_text: Ref,
    font_bold: Ref,
    font_symbol: Ref,
    tick: Ref,  // shared "checked" appearance stream
    blank: Ref, // shared "unchecked" appearance stream
}

impl Sheet {
    fn new() -> Self {
        let mut pdf = Pdf::new();
        pdf.set_version(1, 7);

        let mut sheet = Sheet {
            pdf,
            next_id: 1,
            canvas: Content::new(),
            annots: Vec::new(),
            root_fields: Vec::new(),
            pages: Vec::new(),
            art: Vec::new(),
            page_tree: Ref::new(1),
            font_text: Ref::new(1),
            font_bold: Ref::new(1),
            font_symbol: Ref::new(1),
            tick: Ref::new(1),
            blank: Ref::new(1),
        };

        sheet.page_tree = sheet.alloc();
        sheet.font_text = sheet.alloc();
        sheet.font_bold = sheet.alloc();
        sheet.font_symbol = sheet.alloc();
        sheet.tick = sheet.alloc();
        sheet.blank = sheet.alloc();
        sheet.write_check_appearances();
        sheet
    }

    fn alloc(&mut self) -> Ref {
        let id = Ref::new(self.next_id);
        self.next_id += 1;
        id
    }

    /* -------------------------- drawing -------------------------- */

    fn text(&mut self, x: f32, y: f32, size: f32, bold: bool, gray: f32, s: &str) {
        let font = if bold { F_BOLD } else { F_TEXT };
        self.canvas.begin_text();
        self.canvas.set_fill_gray(gray);
        self.canvas.set_font(font, size);
        self.canvas.next_line(x, y); // absolute: begin_text resets the matrix
        self.canvas.show(Str(s.as_bytes()));
        self.canvas.end_text();
    }

    /// Rough horizontal centring. Helvetica has no width table here, so this
    /// estimates from the glyph count - close enough for button captions.
    fn text_centered(&mut self, cx: f32, y: f32, size: f32, gray: f32, s: &str) {
        let width = s.chars().count() as f32 * size * 0.58;
        self.text(cx - width / 2.0, y, size, true, gray, s);
    }

    /// Tiny upper-case label that sits above an input box.
    fn caption(&mut self, x: f32, y: f32, s: &str) {
        self.text(x, y, 6.2, true, CAPTION_INK, &s.to_uppercase());
    }

    fn line(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, gray: f32, width: f32) {
        self.canvas.set_stroke_gray(gray);
        self.canvas.set_line_width(width);
        self.canvas.move_to(x1, y1);
        self.canvas.line_to(x2, y2);
        self.canvas.stroke();
    }

    fn box_outline(&mut self, x: f32, y: f32, w: f32, h: f32, fill: f32) {
        self.canvas.set_fill_gray(fill);
        self.canvas.set_stroke_gray(RULE);
        self.canvas.set_line_width(0.7);
        self.canvas.rect(x, y, w, h);
        self.canvas.fill_nonzero_and_stroke();
    }

    fn frame(&mut self, x: f32, y: f32, w: f32, h: f32) {
        self.box_outline(x, y, w, h, TINT);
    }

    fn solid(&mut self, x: f32, y: f32, w: f32, h: f32, gray: f32) {
        self.canvas.set_fill_gray(gray);
        self.canvas.rect(x, y, w, h);
        self.canvas.fill_nonzero();
    }

    /// Dark section header bar with reversed-out text.
    fn banner(&mut self, x: f32, y: f32, w: f32, title: &str) {
        self.solid(x, y, w, 15.0, BANNER);
        self.text(x + 6.0, y + 4.6, 9.0, true, 1.0, &title.to_uppercase());
    }

    /* --------------------------- fields -------------------------- */

    /// Plain text input.
    fn input(&mut self, name: &str, x: f32, y: f32, w: f32, h: f32, opt: Input) -> Ref {
        if opt.tinted {
            self.frame(x, y, w, h);
        }

        let id = self.alloc();
        // /DA - how the reader renders whatever the user types.
        let da = format!("/Helv {} Tf 0 g", opt.size);

        let mut field = self.pdf.form_field(id);
        field
            .partial_name(TextStr(name))
            .field_type(FieldType::Text)
            .field_flags(FieldFlags::from_bits_truncate(opt.flags))
            .vartext_default_appearance(Str(da.as_bytes()))
            .vartext_quadding(opt.quad);

        // Terminal field: the field dict and its widget annotation are one and
        // the same object, so we just keep writing into it.
        let mut annot = field.into_annotation();
        annot
            .rect(Rect::new(x, y, x + w, y + h))
            .flags(AnnotationFlags::PRINT);
        annot.finish();

        self.annots.push(id);
        self.root_fields.push(id);
        id
    }

    /// Caption above, input below.
    fn labelled(&mut self, cap: &str, name: &str, x: f32, y: f32, w: f32, h: f32, opt: Input) {
        self.caption(x, y + h + CAP_DY, cap);
        self.input(name, x, y, w, h, opt);
    }

    fn check_box(&mut self, name: &str, x: f32, y: f32) -> Ref {
        // The empty square is drawn on the page itself, so it survives readers
        // that throw our appearance streams away and rebuild their own.
        self.box_outline(x, y, CHECK, CHECK, 1.0);

        let id = self.alloc();
        let (tick, blank) = (self.tick, self.blank);

        let mut field = self.pdf.form_field(id);
        field
            .partial_name(TextStr(name))
            .field_type(FieldType::Button)
            .checkbox_value(CheckBoxState::Off)
            .checkbox_default_value(CheckBoxState::Off)
            // Size 0 = auto fit. This /DA plus the /MK caption below is the
            // combination Acrobat writes, and it is what a reader falls back
            // to when it regenerates the appearance itself.
            .vartext_default_appearance(Str(b"/ZaDb 0 Tf 0 g"));

        let mut annot = field.into_annotation();
        annot
            .rect(Rect::new(x, y, x + CHECK, y + CHECK))
            .flags(AnnotationFlags::PRINT);
        // /AS picks which of the /N sub-appearances is showing right now.
        annot.appearance_state(Name(b"Off"));
        annot
            .appearance_characteristics()
            .normal_caption(TextStr("4"));
        annot
            .appearance()
            .normal()
            .streams()
            .pairs([(Name(b"Yes"), tick), (Name(b"Off"), blank)]);
        annot.finish();

        self.annots.push(id);
        self.root_fields.push(id);
        id
    }

    /// Checkbox with a label to its right. Unused by the current layout -
    /// kept for status ticks, conditions, ammo boxes and the like.
    #[allow(dead_code)]
    fn labelled_check(&mut self, cap: &str, name: &str, x: f32, y: f32) {
        self.check_box(name, x, y);
        let label = cap.to_uppercase();
        self.text(x + CHECK + 4.0, y + 3.0, 6.6, true, CAPTION_INK, &label);
    }

    /// Drop-down. EDIT lets the user also type something not in the list.
    fn combo(
        &mut self,
        cap: &str,
        name: &str,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        options: &[&str],
    ) {
        self.caption(x, y + h + CAP_DY, cap);
        self.frame(x, y, w, h);
        // Little triangle so it reads as a drop-down on paper too.
        self.canvas.set_fill_gray(CAPTION_INK);
        self.canvas.move_to(x + w - 11.0, y + h - 6.0);
        self.canvas.line_to(x + w - 4.0, y + h - 6.0);
        self.canvas.line_to(x + w - 7.5, y + h - 11.0);
        self.canvas.close_path();
        self.canvas.fill_nonzero();

        let id = self.alloc();
        let mut field = self.pdf.form_field(id);
        field
            .partial_name(TextStr(name))
            .field_type(FieldType::Choice)
            .field_flags(FieldFlags::COMBO | FieldFlags::EDIT | FieldFlags::DO_NOT_SPELL_CHECK)
            .vartext_default_appearance(Str(b"/Helv 9 Tf 0 g"));
        field
            .choice_options()
            .options(options.iter().map(|s| TextStr(s)));

        let mut annot = field.into_annotation();
        annot
            .rect(Rect::new(x, y, x + w, y + h))
            .flags(AnnotationFlags::PRINT);
        annot.finish();

        self.annots.push(id);
        self.root_fields.push(id);
    }

    /// Radio set: one parent field + one kid widget per option. Also unused
    /// right now - drop it back in whenever you need an either/or choice.
    ///
    /// Note the `&[u8]` in the option type. Byte-string literals are arrays
    /// (`&[u8; 7]`), so `[b"Classic", b"Pulp"]` refuses to compile because the
    /// lengths differ - annotating the slice as `&[u8]` coerces them, and then
    /// the state names can be any length you like.
    #[allow(dead_code)]
    fn radio_group(&mut self, group: &str, x: f32, y: f32, gap: f32, options: &[(&str, &[u8])]) {
        // Labels first: drawing borrows the canvas, field writing borrows the
        // Pdf, and doing them in separate passes keeps that readable.
        for (i, (label, _)) in options.iter().enumerate() {
            let ox = x + gap * i as f32;
            let label = label.to_uppercase();
            self.box_outline(ox, y, CHECK, CHECK, 1.0);
            self.text(ox + CHECK + 4.0, y + 3.0, 6.6, true, CAPTION_INK, &label);
        }

        let kids: Vec<Ref> = options.iter().map(|_| self.alloc()).collect();
        let parent = self.alloc();

        let mut field = self.pdf.form_field(parent);
        field
            .partial_name(TextStr(group))
            .field_type(FieldType::Button)
            // RADIO            -> mutually exclusive
            // NO_TOGGLE_TO_OFF -> once one is picked you can only move the pick
            // RADIOS_IN_UNISON -> kids sharing an export name toggle together
            .field_flags(
                FieldFlags::RADIO | FieldFlags::NO_TOGGLE_TO_OFF | FieldFlags::RADIOS_IN_UNISON,
            )
            .children(kids.iter().copied());
        field.finish();

        let (tick, blank) = (self.tick, self.blank);
        for (i, (_, on_state)) in options.iter().enumerate() {
            let ox = x + gap * i as f32;
            let mut kid = self.pdf.form_field(kids[i]);
            kid.parent(parent)
                .vartext_default_appearance(Str(b"/ZaDb 0 Tf 0 g"));
            let mut annot = kid.into_annotation();
            annot
                .rect(Rect::new(ox, y, ox + CHECK, y + CHECK))
                .flags(AnnotationFlags::PRINT);
            annot.appearance_state(Name(b"Off"));
            annot
                .appearance_characteristics()
                .normal_caption(TextStr("4"));
            annot
                .appearance()
                .normal()
                .streams()
                .pairs([(Name(on_state), tick), (Name(b"Off"), blank)]);
            annot.finish();
            self.annots.push(kids[i]);
        }

        self.root_fields.push(parent);
    }

    /// Push button. These hold no value of their own, they only fire an
    /// action when clicked.
    fn push_button(
        &mut self,
        name: &str,
        label: &str,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        primary: bool,
        action: ButtonAction,
    ) {
        // Primary = solid, everything else = outlined, so the destructive
        // one does not look like the most inviting thing on the page.
        if primary {
            self.solid(x, y, w, h, 0.15);
            self.text_centered(x + w / 2.0, y + h / 2.0 - 2.6, 7.0, 1.0, label);
        } else {
            self.box_outline(x, y, w, h, 1.0);
            self.text_centered(x + w / 2.0, y + h / 2.0 - 2.6, 7.0, 0.2, label);
        }

        let id = self.alloc();
        let mut field = self.pdf.form_field(id);
        field
            .partial_name(TextStr(name))
            .field_type(FieldType::Button)
            .field_flags(FieldFlags::PUSHBUTTON);

        let mut annot = field.into_annotation();
        annot
            .rect(Rect::new(x, y, x + w, y + h))
            .flags(AnnotationFlags::PRINT);
        annot.appearance_characteristics().border_color_gray(1.0);
        {
            let mut act = annot.action();
            match action {
                // /S /Named has no ActionType variant, but Action derefs to
                // Dict so the two keys can just be written by hand.
                ButtonAction::Named(verb) => {
                    act.pair(Name(b"S"), Name(b"Named"));
                    act.pair(Name(b"N"), Name(verb));
                }
                ButtonAction::ResetFields(ids) => {
                    act.action_type(ActionType::ResetForm);
                    // No INCLUDE_EXCLUDE flag, so /Fields is an include list:
                    // only these fields get cleared. Leaving the flag set with
                    // an empty array is what wipes the entire form.
                    act.fields().ids(ids);
                }
            }
        }
        annot.finish();

        self.annots.push(id);
        self.root_fields.push(id);
    }

    /* ---------------------- composite widgets --------------------- */

    /// A pair of boxes under one caption, e.g. HIT POINTS current / maximum.
    fn paired(&mut self, cap: &str, key: &str, x: f32, y: f32) {
        let big = Input::new().size(11.0).center();
        self.caption(x, y + 20.0 + CAP_DY, cap);
        self.input(&format!("{}_cur", key), x, y, 45.0, 20.0, big);
        self.input(&format!("{}_max", key), x + 47.0, y, 45.0, 20.0, big);
        self.text(x + 12.0, y - 7.5, 5.6, false, CAPTION_INK, "CURRENT");
        self.text(x + 61.0, y - 7.5, 5.6, false, CAPTION_INK, "MAX");
    }

    /// One line of the skill list: [x] Name ............ [value]
    /// `printed = None` leaves a blank row the player names themselves.
    fn skill_row(&mut self, index: usize, printed: Option<&str>, x: f32, y: f32) {
        let key = format!("skill_{:02}", index);
        let num = Input::new().size(8.5).center();

        self.check_box(&format!("{}_used", key), x, y + 1.0);
        match printed {
            Some(label) => {
                self.text(x + 15.0, y + 3.8, 7.3, false, INK, label);
                self.line(x + 15.0, y + 1.6, x + 98.0, y + 1.6, 0.85, 0.5);
            }
            None => {
                let opt = Input::new().size(7.3);
                self.input(&format!("{}_name", key), x + 14.0, y, 86.0, 13.0, opt);
            }
        }
        self.input(&format!("{}_value", key), x + 102.0, y, 26.0, 13.0, num);
    }

    /// Store vector art once as a Form XObject. `draw` paints into a box
    /// whose own coordinate space runs from (0,0) to (w,h), so the art does
    /// not care where it ends up on the page.
    ///
    /// Paths cost a few hundred bytes for the whole drawing, and every extra
    /// placement is about 30 bytes, so the same ornament in four corners of
    /// two pages is stored once and referenced eight times.
    #[allow(dead_code)]
    fn define_art(
        &mut self,
        name: Name<'static>,
        w: f32,
        h: f32,
        draw: impl FnOnce(&mut Content),
    ) -> Art {
        let mut content = Content::new();
        draw(&mut content);
        let body = content.finish();

        let id = self.alloc();
        let mut xobj = self.pdf.form_xobject(id, &body);
        xobj.bbox(Rect::new(0.0, 0.0, w, h));
        xobj.finish();

        self.art.push((name, id));
        Art { name, w, h }
    }

    /// Paint stored art into the given rectangle, scaling to fit it.
    #[allow(dead_code)]
    fn place_art(&mut self, art: Art, x: f32, y: f32, w: f32, h: f32) {
        self.canvas.save_state();
        // maps the art's own box onto the target rect
        self.canvas.transform([w / art.w, 0.0, 0.0, h / art.h, x, y]);
        self.canvas.x_object(art.name);
        self.canvas.restore_state();
    }

    /// A faint frame marking space kept clear for artwork.
    fn art_frame(&mut self, x: f32, y: f32, w: f32, h: f32, label: &str) {
        self.canvas.set_stroke_gray(0.82);
        self.canvas.set_line_width(0.6);
        self.canvas.rect(x, y, w, h);
        self.canvas.stroke();
        self.text(x + 5.0, y + 6.0, 5.4, false, 0.62, label);
    }

    /// Four corner blocks kept free for crests, sigils, tape, whatever.
    fn corner_art(&mut self) {
        for (x, y) in [
            (CORNER_X, CORNER_TOP_Y),
            (PAGE_W - CORNER_X - CORNER_W, CORNER_TOP_Y),
            (CORNER_X, CORNER_BOTTOM_Y),
            (PAGE_W - CORNER_X - CORNER_W, CORNER_BOTTOM_Y),
        ] {
            self.art_frame(x, y, CORNER_W, CORNER_H, "ART");
        }
    }

    /// Reserved block for a body silhouette with the armour boxes laid out
    /// where the body parts will be once the artwork sits behind them.
    ///
    /// The drawing itself is deliberately left empty. To fill it, define the
    /// figure once and place it here:
    ///
    /// ```ignore
    /// let figure = s.define_art(Name(b"Figure"), 131.0, 336.0, |c| {
    ///     c.set_line_width(1.4);
    ///     c.set_stroke_gray(0.35);
    ///     c.move_to(65.0, 56.0);
    ///     c.line_to(65.0, 200.0);
    ///     c.stroke();
    ///     // ... the rest of the outline
    /// });
    /// s.place_art(figure, ax, 96.0, 131.0, 336.0);
    /// ```
    ///
    /// Place it before the boxes below so the numbers stay on top. For a
    /// photograph rather than line art, `pdf.image_xobject(id, samples)`
    /// takes the same route - but see the notes on file size first.
    fn armour_panel(&mut self, x: f32, y: f32, w: f32, h: f32) {
        self.art_frame(x, y, w, h, "ARTWORK AREA");

        let hit = Input::new().size(9.5).center();
        let cx = x + w / 2.0;
        let top = y + h;

        self.labelled("Head", "armour_head", cx - 22.0, top - 54.0, 44.0, 18.0, hit);

        let mid = top - 132.0;
        self.labelled("L arm", "armour_arm_l", x + 2.0, mid, 40.0, 18.0, hit);
        self.labelled("Body", "armour_body", cx - 21.0, mid, 42.0, 18.0, hit);
        self.labelled("R arm", "armour_arm_r", x + w - 42.0, mid, 40.0, 18.0, hit);

        let low = top - 232.0;
        self.labelled("L leg", "armour_leg_l", cx - 46.0, low, 42.0, 18.0, hit);
        self.labelled("R leg", "armour_leg_r", cx + 4.0, low, 42.0, 18.0, hit);
    }

    /// Caption + multi-line box for the prose sections.
    fn note_block(&mut self, cap: &str, name: &str, x: f32, y: f32, w: f32, h: f32) {
        self.labelled(cap, name, x, y, w, h, Input::new().size(8.5).multiline());
    }

    /* ------------------------- page plumbing ---------------------- */

    fn footer(&mut self, page_no: usize, title: &str) {
        self.line(PLATE_X, 66.0, PLATE_R, 66.0, 0.8, 0.5);
        let label = format!("{}  -  PAGE {} OF {}", title.to_uppercase(), page_no, PAGES);
        self.text(PLATE_X, 55.0, 6.4, true, CAPTION_INK, &label);

        // Navigation and save only. Nothing down here can destroy work.
        //
        // If you ever do want a per-section clear, `ButtonAction::ResetFields`
        // is still wired up: pass it the ids of the block you want emptied,
        // e.g. `ButtonAction::ResetFields(vec![hp_cur, hp_max, san_cur, ..])`.
        // Cost is about 250 bytes per button, so a dozen of them would not
        // meaningfully change the file size.
        let (h, gap) = (18.0, 6.0);
        let widths = [46.0, 46.0, 52.0];
        let total: f32 = widths.iter().sum::<f32>() + gap * 2.0;
        let mut x = PLATE_R - total;

        self.push_button(
            &format!("back_p{}", page_no), "BACK", x, 44.0, widths[0], h, false,
            ButtonAction::Named(b"PrevPage"),
        );
        x += widths[0] + gap;
        self.push_button(
            &format!("fwd_p{}", page_no), "NEXT", x, 44.0, widths[1], h, false,
            ButtonAction::Named(b"NextPage"),
        );
        x += widths[1] + gap;
        self.push_button(
            &format!("save_p{}", page_no), "SAVE", x, 44.0, widths[2], h, true,
            ButtonAction::Named(b"SaveAs"),
        );
    }

    fn end_page(&mut self) {
        let page_id = self.alloc();
        let content_id = self.alloc();
        let body = std::mem::replace(&mut self.canvas, Content::new()).finish();
        let annots = std::mem::take(&mut self.annots);

        let (tree, ft, fb, fs) =
            (self.page_tree, self.font_text, self.font_bold, self.font_symbol);
        let art = self.art.clone();
        let mut page = self.pdf.page(page_id);
        page.media_box(Rect::new(0.0, 0.0, PAGE_W, PAGE_H))
            .parent(tree)
            .contents(content_id);
        {
            let mut res = page.resources();
            res.fonts().pairs([(F_TEXT, ft), (F_BOLD, fb), (F_SYMBOL, fs)]);
            if !art.is_empty() {
                let mut xobjects = res.x_objects();
                for (name, id) in &art {
                    xobjects.pair(*name, *id);
                }
            }
            res.finish();
        }
        page.annotations(annots);
        page.finish();

        // Flate the drawing instructions. They are long and very repetitive,
        // so this is roughly a 5x saving on the biggest part of the file.
        let body = miniz_oxide::deflate::compress_to_vec_zlib(&body, 8);
        self.pdf
            .stream(content_id, &body)
            .filter(Filter::FlateDecode)
            .finish();
        self.pages.push(page_id);
    }

    /// The two shared appearance streams every checkbox and radio reuses.
    fn write_check_appearances(&mut self) {
        let bbox = Rect::new(0.0, 0.0, CHECK, CHECK);

        let mut on = Content::new();
        on.save_state();
        on.set_line_width(0.8);
        on.set_stroke_gray(RULE);
        on.set_fill_gray(1.0);
        on.rect(0.4, 0.4, CHECK - 0.8, CHECK - 0.8);
        on.fill_nonzero_and_stroke();
        on.begin_text();
        on.set_fill_gray(INK);
        on.set_font(F_SYMBOL, 9.0); // ZapfDingbats: '4' is a check mark
        on.next_line(1.3, 1.9);
        on.show(Str(b"4"));
        on.end_text();
        on.restore_state();
        let on = on.finish();

        let (id, symbol) = (self.tick, self.font_symbol);
        let mut xobj = self.pdf.form_xobject(id, &on);
        xobj.bbox(bbox);
        xobj.resources().fonts().pair(F_SYMBOL, symbol);
        xobj.finish();

        let mut off = Content::new();
        off.save_state();
        off.set_line_width(0.8);
        off.set_stroke_gray(RULE);
        off.set_fill_gray(1.0);
        off.rect(0.4, 0.4, CHECK - 0.8, CHECK - 0.8);
        off.fill_nonzero_and_stroke();
        off.restore_state();
        let off = off.finish();

        let id = self.blank;
        let mut xobj = self.pdf.form_xobject(id, &off);
        xobj.bbox(bbox);
        xobj.finish();
    }

    fn finish(mut self) -> Vec<u8> {
        let info_id = self.alloc();
        self.pdf
            .document_info(info_id)
            .title(TextStr("Investigator Sheet (template)"))
            .creator(TextStr("pdf-writer"));

        // Base-14 fonts: nothing to embed, nothing to license.
        let (ft, fb, fs) = (self.font_text, self.font_bold, self.font_symbol);
        self.pdf.type1_font(ft).base_font(Name(b"Helvetica"));
        self.pdf.type1_font(fb).base_font(Name(b"Helvetica-Bold"));
        self.pdf.type1_font(fs).base_font(Name(b"ZapfDingbats"));

        let catalog_id = self.alloc();
        let tree = self.page_tree;
        let fields = std::mem::take(&mut self.root_fields);

        let mut catalog = self.pdf.catalog(catalog_id);
        catalog.pages(tree);
        {
            let mut form = catalog.form();
            form.fields(fields);
            form.default_appearance(Str(b"/Helv 9 Tf 0 g"));
            form.quadding(Quadding::Left);
            // We only ship appearance streams for the tick boxes, so ask the
            // reader to build the rest itself. Without this, some viewers only
            // show typed text while the field has focus.
            form.pair(Name(b"NeedAppearances"), true);
            form.default_resources()
                .fonts()
                .pairs([(F_TEXT, ft), (F_BOLD, fb), (F_SYMBOL, fs)]);
        }
        catalog.finish();

        let pages = std::mem::take(&mut self.pages);
        let count = pages.len() as i32;
        self.pdf.pages(tree).kids(pages).count(count);

        self.pdf.finish()
    }
}

/* ------------------------------------------------------------------ *
 * Content tables - edit these, not the layout code.
 * ------------------------------------------------------------------ */

const CHARACTERISTICS: [(&str, &str); 8] = [
    ("str", "STR"),
    ("con", "CON"),
    ("siz", "SIZ"),
    ("dex", "DEX"),
    ("app", "APP"),
    ("pow", "POW"),
    ("int", "INT"),
    ("edu", "EDU"),
];

/// Rows per skill column. Three columns, so the table below holds 3x this.
const SKILL_ROWS: usize = 22;

/// Column-major: fills column 1 top to bottom, then column 2, then column 3.
/// `None` leaves a blank row whose name the player types in.
const SKILLS: [Option<&str>; SKILL_ROWS * 3] = [
    // ---- column 1 -----------------------------------------------------
    Some("Accounting (05)"),
    Some("Anthropology (01)"),
    Some("Appraise (05)"),
    Some("Archaeology (01)"),
    Some("Art/Craft ... (05)"),
    Some("Charm (15)"),
    Some("Climb (20)"),
    Some("Credit Rating (00)"),
    Some("Cthulhu Mythos (00)"),
    Some("Disguise (05)"),
    Some("Dodge (DEX/2)"),
    Some("Drive Auto (20)"),
    Some("Electrical Repair (10)"),
    Some("Fast Talk (05)"),
    Some("Fighting (Brawl) (25)"),
    Some("Fighting ... (01)"),
    Some("Firearms (Handgun) (20)"),
    Some("Firearms (Rifle/SG) (25)"),
    Some("First Aid (30)"),
    Some("History (05)"),
    Some("Intimidate (15)"),
    Some("Jump (20)"),
    // ---- column 2 -----------------------------------------------------
    Some("Language Own (EDU)"),
    Some("Language ... (01)"),
    Some("Language ... (01)"),
    Some("Law (05)"),
    Some("Library Use (20)"),
    Some("Listen (20)"),
    Some("Locksmith (01)"),
    Some("Lore ... (01)"),
    Some("Mech. Repair (10)"),
    Some("Medicine (01)"),
    Some("Natural World (10)"),
    Some("Navigate (10)"),
    Some("Occult (05)"),
    Some("Op. Hvy. Machine (01)"),
    Some("Persuade (10)"),
    Some("Pilot ... (01)"),
    Some("Psychoanalysis (01)"),
    Some("Psychology (10)"),
    Some("Ride (05)"),
    Some("Science ... (01)"),
    Some("Science ... (01)"),
    Some("Sleight of Hand (10)"),
    // ---- column 3 -----------------------------------------------------
    Some("Spot Hidden (25)"),
    Some("Stealth (20)"),
    Some("Survival ... (10)"),
    Some("Swim (20)"),
    Some("Throw (20)"),
    Some("Track (10)"),
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
];

const WEAPON_COLS: [(&str, f32); 9] = [
    ("Weapon", 148.0),
    ("Regular", 44.0),
    ("Hard", 44.0),
    ("Extreme", 44.0),
    ("Damage", 74.0),
    ("Range", 50.0),
    ("Atks", 40.0),
    ("Ammo", 44.0),
    ("Malf", 47.0),
];

const TABLE_ROWS: usize = 5;

/// Width of the three skill columns together (3 x 128 + two 6pt gutters).
const SKILL_BLOCK_W: f32 = 396.0;

const ARCHETYPES: [&str; 10] = [
    "Adventurer",
    "Bon Vivant",
    "Egghead",
    "Explorer",
    "Grease Monkey",
    "Hard Boiled",
    "Mystic",
    "Scholar",
    "Sidekick",
    "Two-Fisted",
];

/* ------------------------------------------------------------------ *
 * Page 1 - everything you touch during play.
 * ------------------------------------------------------------------ */

fn page_front(s: &mut Sheet) {
    s.corner_art();
    s.banner(PLATE_X, 800.0, PLATE_W, "Investigator record");
    s.text(PLATE_R - 62.0, 804.8, 6.4, true, 1.0, "TEMPLATE  v0.3");

    // Identity - three rows, pitch 28
    let row = Input::new();
    s.labelled("Investigator name", "name", MARGIN, 766.0, 250.0, BOX_H, row);
    s.labelled("Player", "player", 288.0, 766.0, 130.0, BOX_H, row);
    s.labelled("Age", "age", 426.0, 766.0, 40.0, BOX_H, row.center());
    s.labelled("Sex / pronouns", "sex", 474.0, 766.0, 91.0, BOX_H, row);

    s.labelled("Occupation", "occupation", MARGIN, 738.0, 250.0, BOX_H, row);
    s.combo("Archetype", "archetype", 288.0, 738.0, 130.0, BOX_H, &ARCHETYPES);
    s.labelled("Birthplace", "birthplace", 426.0, 738.0, 139.0, BOX_H, row);

    s.labelled("Residence", "residence", MARGIN, 710.0, 250.0, BOX_H, row);
    s.labelled("Spending level", "spending", 288.0, 710.0, 130.0, BOX_H, row);
    s.labelled("Cash & assets", "cash", 426.0, 710.0, 139.0, BOX_H, row);

    // Characteristics - one strip, one value each
    s.banner(MARGIN, 684.0, FULL_W, "Characteristics");
    let stat = Input::new().size(12.0).center();
    for (i, (key, label)) in CHARACTERISTICS.iter().enumerate() {
        let x = MARGIN + 68.1 * i as f32;
        s.text(x + 2.0, 677.0, 8.0, true, INK, label);
        s.input(key, x, 652.0, 58.0, 22.0, stat);
    }

    // Derived numbers
    s.banner(MARGIN, 626.0, FULL_W, "Health, luck & combat");
    s.paired("Hit points", "hp", MARGIN, 586.0);
    s.paired("Magic points", "mp", 128.0, 586.0);
    s.paired("Sanity", "san", 226.0, 586.0);
    let mid = Input::new().size(11.0).center();
    s.labelled("Luck", "luck", 324.0, 586.0, 60.0, 20.0, mid);
    s.labelled("Move", "move", 390.0, 586.0, 52.0, 20.0, mid);
    s.labelled("Build", "build", 448.0, 586.0, 52.0, 20.0, mid);
    s.labelled("Dmg bonus", "db", 506.0, 586.0, 58.0, 20.0, mid);

    // Weapons
    s.banner(MARGIN, 556.0, FULL_W, "Combat");
    let head_y = 538.0;
    let table_bottom = head_y - 16.0 * TABLE_ROWS as f32;
    s.solid(MARGIN, head_y, FULL_W, 14.0, 0.86);
    let mut x = MARGIN;
    for (title, w) in WEAPON_COLS.iter() {
        let head = title.to_uppercase();
        s.text(x + 4.0, head_y + 4.4, 6.4, true, CAPTION_INK, &head);
        s.line(x, head_y, x, table_bottom, RULE, 0.5);
        x += w;
    }
    s.line(RIGHT, head_y, RIGHT, table_bottom, RULE, 0.5);
    s.line(MARGIN, head_y, RIGHT, head_y, RULE, 0.5);

    let cell = Input::new().size(8.0).bare();
    for r in 0..TABLE_ROWS {
        let y = head_y - 16.0 * (r + 1) as f32;
        s.line(MARGIN, y, RIGHT, y, RULE, 0.5);
        let mut x = MARGIN;
        for (c, (_, w)) in WEAPON_COLS.iter().enumerate() {
            let opt = if c == 0 { cell } else { cell.center() };
            s.input(&format!("weapon_{}_{}", r, c), x + 1.0, y + 1.0, w - 2.0, 14.0, opt);
            x += w;
        }
    }

    // Skills - three 128pt columns, one value box each. What the half and
    // fifth boxes used to take up is now the armour panel on the right.
    s.banner(MARGIN, 438.0, SKILL_BLOCK_W, "Skills");
    let cols = [MARGIN, 164.0, 298.0];
    for x in cols.iter() {
        s.text(x + 102.0, 433.0, 5.8, true, CAPTION_INK, "VALUE");
    }
    for (i, entry) in SKILLS.iter().enumerate() {
        let x = cols[i / SKILL_ROWS];
        let y = 414.0 - 16.0 * (i % SKILL_ROWS) as f32;
        s.skill_row(i, *entry, x, y);
    }

    // Armour - reserved artwork block plus the hit-location boxes
    let ax = MARGIN + SKILL_BLOCK_W + 8.0;
    let aw = RIGHT - ax;
    s.banner(ax, 438.0, aw, "Armour");
    s.armour_panel(ax, 96.0, aw, 336.0);

    s.footer(1, "Investigator sheet");
    s.end_page();
}

/* ------------------------------------------------------------------ *
 * Page 2 - who they are and what they carry.
 * ------------------------------------------------------------------ */

fn page_notes(s: &mut Sheet) {
    s.corner_art();
    s.banner(PLATE_X, 800.0, PLATE_W, "Backstory & possessions");
    s.labelled("Investigator", "name_p2", MARGIN, 766.0, 300.0, BOX_H, Input::new());
    s.labelled("Player", "player_p2", 340.0, 766.0, 225.0, BOX_H, Input::new());

    let left = MARGIN;
    let right = 305.0;
    let w = 260.0;
    let h = 57.0;
    let pitch = 75.0;

    let left_blocks = [
        ("Personal description", "desc"),
        ("Ideology & beliefs", "ideology"),
        ("Significant people", "people"),
        ("Meaningful locations", "locations"),
        ("Treasured possessions", "treasures"),
        ("Traits", "traits"),
    ];
    let right_blocks = [
        ("Injuries & scars", "injuries"),
        ("Phobias & manias", "phobias"),
        ("Arcane tomes, spells & artifacts", "tomes"),
        ("Encounters with strange entities", "entities"),
        ("Fellow investigators", "party"),
        ("Armour & combat notes", "armour"),
    ];
    for (i, (cap, key)) in left_blocks.iter().enumerate() {
        s.note_block(cap, key, left, 696.0 - pitch * i as f32, w, h);
    }
    for (i, (cap, key)) in right_blocks.iter().enumerate() {
        s.note_block(cap, key, right, 696.0 - pitch * i as f32, w, h);
    }

    s.banner(MARGIN, 294.0, FULL_W, "Gear, talents & wealth");
    s.note_block("Gear & possessions", "gear", left, 180.0, w, 95.0);
    s.note_block("Talents & special abilities", "talents", right, 180.0, w, 95.0);
    s.note_block("Assets, property & income", "assets", MARGIN, 88.0, FULL_W, 80.0);

    s.footer(2, "Investigator sheet");
    s.end_page();
}

fn main() -> std::io::Result<()> {
    let mut sheet = Sheet::new();
    page_front(&mut sheet);
    page_notes(&mut sheet);

    std::fs::create_dir_all("target")?;
    std::fs::write("target/investigator_sheet.pdf", sheet.finish())
}
