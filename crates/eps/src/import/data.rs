//! Data the program reads: from itself (`currentfile`) through decoding filters, and images
//! sampled from it, from strings or from procedures.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use hayro_ccitt::{DecodeSettings, Decoder, DecoderContext, EncodingMode};
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::cmyk::{Inks, jpeg_components};
use vectorcraft_doc::{ImageBlob, ImageObject, Node, NodeId, NodeKind, OpacityMask};
use vectorcraft_geom::Affine;

use super::graphics::{Space, process};
use super::interp::{Interp, matrix_of};
use super::lex::{Lexer, find, hex_decode};
use super::obj::{DictRef, Key, Obj, Op, PsError, Res, Shared, ps_err};
use crate::ps;

mod filters;
mod ops;
mod pixels;

pub(crate) use filters::Stream;
