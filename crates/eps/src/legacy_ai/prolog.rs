use super::*;

/// The setup's fonts (§3.1, §3.3): `%%IncludeFont` for each, the encoding strings are in (`TE`),
/// and each font re-encoded to it as `_name` (`TZ`; a TrueType font takes the encoding as its own,
/// not the standard one, as the specification says of fonts that aren't Type 1).
pub(super) fn fonts(s: &mut String, fonts: &[(String, bool)]) {
    if fonts.is_empty() {
        return;
    }
    for (name, _) in fonts {
        let _ = writeln!(s, "%%IncludeFont: {name}");
    }
    let te = latin::te();
    s.push_str(&te);
    // The encoding pairs, without `[` and `TE`.
    let pairs = te.trim_start_matches('[').trim_end().trim_end_matches("TE").trim_end();
    for (name, truetype) in fonts {
        let _ = writeln!(s, "%AI3_BeginEncoding: _{name} {name}");
        if *truetype {
            let _ = writeln!(s, "[{pairs}\n/_{name}/{name} 0 0 0 TZ\n%AI3_EndEncoding TrueType");
        } else {
            let _ = writeln!(s, "[/_{name}/{name} 0 0 1 TZ\n%AI3_EndEncoding AdobeType");
        }
    }
}

/// The prolog's procedure set, as `%%DocumentSuppliedResources` names it: a file that has it is
/// read from its program when its layers are asked for ([`crate::import_with`]).
pub(crate) const PROCSET: &str = "VectorCraft_Legacy 1 0";

/// The prolog: a procedure set of this writer's own (`VectorCraft_Legacy 1 0`), in a dictionary
/// (`VCLegacy`) the setup begins and the trailer ends (§1.2: the prolog holds definitions only,
/// the setup activates them, the trailer deactivates them). It gives each operator the script
/// uses its meaning in PostScript, so that the file prints and previews as a PostScript program,
/// written from the PostScript Language Reference and this format's specification (no other
/// application's procedure set): paths and their painting with the fill and stroke colours,
/// compound paths painted as one path, clipping groups, guides that don't print, layers that are
/// hidden or don't print marking nothing, and images drawn from the samples on the comment lines
/// that follow `XI`. Gradients fill with the colour set before them (the writer sets the first
/// stop's), and type marks nothing: apps that read the format draw both. The page trailer's
/// `annotatepage` (§3, Script Setup: `gsave annotatepage grestore showpage`) marks nothing.
pub(super) const PROLOG: &str = "%%BeginResource: procset VectorCraft_Legacy 1 0
/VCLegacy 96 dict def
VCLegacy begin
/bd {bind def} bind def
% State: the fill and stroke colours (procedures that set them), their overprints, the fill rule,
% a clip to make at the next painting operator, a compound path being built, a guide being
% drawn, whether the layer prints, and what the compound path is painted with.
/_fc {0 setgray} def /_sc {0 setgray} def
/_fo false def /_so false def /_eo false def /_w false def /_cp false def /_g false def
/_on true def /_rf false def /_rs false def
% Painting: a fill and a stroke that keep the path; the path painted as _rf and _rs say (after
% the clip W asked for), then ended.
/_fill {gsave _fc _fo setoverprint _eo {eofill} {fill} ifelse grestore} bd
/_stroke {gsave _sc _so setoverprint stroke grestore} bd
/_draw {_w {_eo {eoclip} {clip} ifelse /_w false def} if
 _on _g not and {_rf {_fill} if _rs {_stroke} if} if newpath} bd
% close? fill? stroke? _r: paints now, or at the end of the compound path being built.
/_r {/_rs exch def /_rf exch def {closepath} if _cp not {_draw} if} bd
% Path construction (section 5.3.2) and painting (5.3.3).
/m {moveto} bd /l {lineto} bd /L {lineto} bd /c {curveto} bd /C {curveto} bd
/N {false false false _r} bd /n {true false false _r} bd
/F {false true false _r} bd /f {true true false _r} bd
/S {false false true _r} bd /s {true false true _r} bd
/B {false true true _r} bd /b {true true true _r} bd
/XR {1 eq /_eo exch def} bd
% Compound paths (5.3.5): one path, painted at its end.
/*u {/_cp true def} bd /*U {/_cp false def _draw} bd
% Groups, clipping groups and masks (5.6), locking (5.1), guides (6.1).
/u {} bd /U {} bd /q {gsave} bd /Q {grestore} bd
/H {} bd /h {closepath} bd /W {/_w true def} bd
/A {pop} bd
/* {pop _g {newpath /_g false def} {/_g true def} ifelse} bd
% Colours (5.4): fill and stroke colours, overprints; a custom colour as its CMYK equivalent at
% its tint (tint 0 is the full colour).
/g {/setgray load 2 array astore cvx /_fc exch def} bd
/G {/setgray load 2 array astore cvx /_sc exch def} bd
/k {/setcmykcolor load 5 array astore cvx /_fc exch def} bd
/K {/setcmykcolor load 5 array astore cvx /_sc exch def} bd
/Xa {/setrgbcolor load 4 array astore cvx /_fc exch def} bd
/XA {/setrgbcolor load 4 array astore cvx /_sc exch def} bd
/_tint {1 exch sub /_t exch def pop 4 {_t mul 4 1 roll} repeat} bd
/x {_tint k} bd /X {_tint K} bd
/O {1 eq /_fo exch def} bd /R {1 eq /_so exch def} bd
/w {setlinewidth} bd /J {setlinecap} bd /j {setlinejoin} bd /M {setmiterlimit} bd /d {setdash} bd
% Gradients (3.5): definitions are skipped; an instance fills with the colour set before it
% (the writer sets its first stop's), and BB strokes the path as its flag says.
/Bn {pop} bd /Bd {pop pop pop} bd /BD {cleartomark} bd
/Bb {gsave} bd /Bg {12 {pop} repeat} bd /Bh {pop pop pop pop} bd
/BB {grestore _cp {0 gt {/_rs true def} if}
 {dup 2 eq {closepath} if 0 gt _on and {_stroke} if newpath} ifelse} bd
% Layers (9): a layer that is hidden or doesn't print marks nothing.
/Lb {6 {pop} repeat exch pop exch pop 1 eq exch 1 eq and /_on exch def} bd
/Ln {pop} bd /LB {/_on true def} bd
% Images (8): the samples, in hex on the comment lines that follow XI, read a line at a time.
/_l 256 string def /_s 128 string def
/_row {{currentfile _l readline not {exit} if dup length 1 gt {exit} if pop} loop
 dup length 1 gt {dup length 1 sub 1 exch getinterval /ASCIIHexDecode filter _s readstring pop} {pop ()} ifelse} bd
/XI {pop pop pop pop /_ic exch def /_ib exch def /_ih exch def /_iw exch def pop pop pop pop
 [1 0 0 -1 0 0] exch matrix concatmatrix
 _on {gsave concat _iw _ih _ib [1 0 0 1 0 0] /_row load false _ic 1 eq {1} {_ic} ifelse colorimage grestore} {pop} ifelse} bd
% Type (15) is set by the app that reads the file: its operators take their operands and mark
% nothing; encodings (3.3) are skipped.
/TE {cleartomark} bd /TZ {cleartomark} bd
/To {pop} bd /Tp {7 {pop} repeat} bd /TP {} bd /TO {} bd
/Tr {pop} bd /Tf {pop pop pop pop} bd /Ts {pop} bd /Tz {pop pop} bd /Tt {pop} bd /TA {pop} bd
/Tl {pop pop} bd /Ta {pop} bd /Ti {pop pop pop} bd /Tx {pop} bd
% The page trailer (3): annotatepage marks nothing.
/annotatepage {} bd
end
%%EndResource
";

/// The first line of a PostScript file (the DSC version comment).
pub(super) const PS_HEADER: &str = "%!PS-Adobe-3.0"; // brand-ok: the PostScript header comment
