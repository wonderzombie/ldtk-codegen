use std::collections::HashMap;

pub use crate::schema::*;
pub use anyhow::*;
pub use codegen::{Block, Scope};
pub use convert_case::Case;
pub use itertools::Itertools;
pub use num::Float;
pub use paste::paste;

pub struct Preferences {
    pub preserve_case: bool,
    pub serde: bool,
    pub vector: Option<String>,
    pub color: Option<String>,
}

impl Preferences {
    pub fn to_case(&self, str: &str, case: Case) -> String {
        if self.preserve_case {
            str.to_owned()
        } else {
            use convert_case::Casing;
            str.to_case(case)
        }
    }
}

pub fn format_color(color: &str) -> Result<String> {
    color
        .strip_prefix('#')
        .map(|color| format!("<Color as ColorImpl>::from_hex(0x{color}FF)"))
        .context("Color should start with #!")
}

#[derive(Default)]
pub struct RsDefinitions {
    pub tilesets: HashMap<i64, RsTilesetDefinition>,
    pub layers: HashMap<String, RsLayerDefinition>,
    pub entities: HashMap<String, RsEntityDefinition>,
    pub level: RsLevelDefinition,

    pub entity_instances: HashMap<String, RsEntityInstance>,
}

#[derive(Default)]
pub struct RsLevelDefinition {
    pub fields: HashMap<String, RsFieldType>,
}

pub struct RsTilesetDefinition {
    pub tile_size: u32,
}

#[derive(Default)]
pub struct RsEntityDefinition {
    pub fields: HashMap<String, RsFieldType>,
}

pub struct RsEntityInstance {
    pub level: usize,
    pub layer: usize,
    pub entity: usize,
}

// * ------------------------------------ Layers ------------------------------------ * //
pub enum RsLayerDefinition {
    IntGrid(RsIntGridDefinition),
    Tiles(RsTilesDefinition),
    Entities,
}

pub struct RsIntGridDefinition {
    pub grid_size: u32,
    pub tile_enum: String,
    pub tile_variants: HashMap<i64, String>,
    pub auto_layer: Option<RsAutoLayerDefinition>,
}
pub struct RsTilesDefinition {
    pub grid_size: u32,
}

pub struct RsAutoLayerDefinition {}

// * ------------------------------------ Fields ------------------------------------ * //
pub enum RsFieldType {
    Option(Box<RsFieldType>),
    Array(Box<RsFieldType>),
    Enum(String),
    Int,
    Float,
    String,
    Bool,
    Color,
    Point,
    Tile,
    FilePath,
    EntityRef,
}

impl RsFieldType {
    pub fn parse(field: &FieldDefinition) -> Result<Self> {
        fn parse_field_definition(field_type: &str, can_be_null: bool) -> Result<RsFieldType> {
            let rs_type = if let Some(enumeration) = field_type.strip_prefix("LocalEnum.") {
                RsFieldType::Enum(enumeration.to_owned())
            } else if let Some(_enumeration) = field_type.strip_prefix("ExternEnum.") {
                // TODO: External enums
                bail!("External enums are not supported yet.");
            } else {
                match field_type {
                    "Int" => RsFieldType::Int,
                    "Float" => RsFieldType::Float,
                    "String" | "Multilines" => RsFieldType::String,
                    "Bool" => RsFieldType::Bool,
                    "Color" => RsFieldType::Color,
                    "Point" => RsFieldType::Point,
                    "Tile" => RsFieldType::Tile,
                    "FilePath" => RsFieldType::FilePath,
                    "EntityRef" => RsFieldType::EntityRef,
                    _ => bail!("Unknown or unsupported field type: '{}'!", field_type),
                }
            };
            Ok(if can_be_null {
                RsFieldType::Option(Box::new(rs_type))
            } else {
                rs_type
            })
        }

        Ok(
            if let Some(generic) = field
                .field_definition_type
                .strip_prefix("Array<")
                .and_then(|postfix| postfix.strip_suffix('>'))
            {
                let generic = parse_field_definition(generic, field.can_be_null)?;
                RsFieldType::Array(Box::new(generic))
            } else {
                parse_field_definition(&field.field_definition_type, field.can_be_null)?
            },
        )
    }

    pub fn string_type(&self) -> String {
        match self {
            RsFieldType::Option(generic) => format!("Option<{}>", generic.string_type()),
            RsFieldType::Array(generic) => format!("Vec<{}>", generic.string_type()),
            RsFieldType::Enum(name) => name.clone(),
            RsFieldType::Int => "i32".to_owned(),
            RsFieldType::Float => "f32".to_owned(),
            RsFieldType::String => "String".to_owned(),
            RsFieldType::Bool => "bool".to_owned(),
            RsFieldType::Color => "Color".to_owned(),
            RsFieldType::Point => "UVec2".to_owned(),
            RsFieldType::Tile => "(TilesetID, UVec2)".to_owned(),
            RsFieldType::FilePath => "std::path::PathBuf".to_owned(),
            RsFieldType::EntityRef => "EntityRef".to_owned(),
        }
    }

    pub fn fmt_value(
        &self,
        definitions: &RsDefinitions,
        value: Option<&serde_json::Value>,
    ) -> Result<String> {
        if let RsFieldType::Option(generic) = self {
            return Ok(if let Some(value) = value {
                format!("Some({})", generic.fmt_value(definitions, Some(value))?)
            } else {
                "None".to_owned()
            });
        }
        let value = value.context("Mandatory object can't be null!")?;

        macro_rules! primitive {
            ($fn:ident, $expectation:literal) => {
                value.$fn().context(format!(
                    concat!("Expected ", $expectation, ", found {}!"),
                    value
                ))?
            };
        }

        macro_rules! object_i64 {
            ($object:ident.$field:ident) => {
                $object
                    .get(stringify!($field))
                    .context(format!(
                        concat!(
                            "Object should contain ",
                            stringify!($field),
                            "! Object: {:?}."
                        ),
                        $object
                    ))?
                    .as_i64()
                    .context(concat!(stringify!($field), " should be integer!"))?
            };
        }

        Ok(match self {
            RsFieldType::Option(_) => bail!("Unreachable: Option is already filtered out!"),
            RsFieldType::Array(generic) => {
                let array = primitive!(as_array, "array");
                let mut elements = Vec::with_capacity(array.len());
                for element in array {
                    elements.push(generic.fmt_value(definitions, Some(element))?);
                }
                format!("vec![{}]", elements.join(", "))
            }
            RsFieldType::Enum(name) => format!("{}::{}", name, primitive!(as_str, "enum variant")),
            RsFieldType::Int => primitive!(as_i64, "integer").to_string(),
            RsFieldType::Float => {
                let value = primitive!(as_f64, "float");
                format!("{:.1$}", value, value.fract().to_string().len().max(3) - 2)
            }
            RsFieldType::String => format!("\"{}\".to_owned()", primitive!(as_str, "string")),
            RsFieldType::Bool => primitive!(as_bool, "bool").to_string(),
            RsFieldType::Color => format_color(primitive!(as_str, "color"))?,
            RsFieldType::Point => {
                let point = primitive!(as_object, "GridPoint");
                format!(
                    "<UVec2 as VectorImpl>::new({} as _, {} as _)",
                    object_i64!(point.cx),
                    object_i64!(point.cy),
                )
            }
            RsFieldType::Tile => {
                let tile = primitive!(as_object, "TilesetRect");
                let tileset_id = object_i64!(tile.tilesetUid);
                let tileset = definitions
                    .tilesets
                    .get(&tileset_id)
                    .context("Tile field tileset was not found!")?;
                format!(
                    "({}, <UVec2 as VectorImpl>::new({} as _, {} as _))",
                    tileset_id,
                    object_i64!(tile.x) as u32 / tileset.tile_size,
                    object_i64!(tile.y) as u32 / tileset.tile_size,
                )
            }
            RsFieldType::FilePath => format!("\"{}\".into()", primitive!(as_str, "filepath")),
            RsFieldType::EntityRef => {
                let entity_ref = primitive!(as_object, "EntityRef");
                let entity_iid = entity_ref
                    .get("entityIid")
                    .context(format!(
                        "Object should contain entityIid! Object: {:?}.",
                        entity_ref
                    ))?
                    .as_str()
                    .context(concat!(stringify!(entityIid), " should be an IID!"))?;

                let entity_ref = definitions
                    .entity_instances
                    .get(entity_iid)
                    .context("EntityRef field points to non-existing entity!")?;
                format!(
                    "EntityRef::new({}, {}, {})",
                    entity_ref.level, entity_ref.layer, entity_ref.entity
                )
            }
        })
    }
}

// * ------------------------------------ Macros ------------------------------------ * //
#[macro_export]
macro_rules! derive_rust_object {
    ($object:ident $serde:expr, $($trait:ident),* $(!partial $($partial_trait:ident),*)?) => {
        if $serde {
            $object.derive("serde::Serialize");
            $object.derive("serde::Deserialize");
        }
        $object.derive("Clone");
        $object.derive("Debug");
        $($object.derive(stringify!($trait));)*
        $($(
            paste! {
                $object.derive(stringify!([<Partial $partial_trait>]));
                $object.derive(stringify!($partial_trait));
            }
        )*)?
    };
}

#[macro_export]
macro_rules! generate_impl {
    ($code:ident $(trait $trait:literal for)? $object:expr => {
        $(const $const:ident: $const_type:ty = $const_val:expr;)*
        $(type $type:ident = $type_val:expr;)*
        $(get $get:ident () -> $get_ret:ty = $get_value:expr;)*
        $(fn $fn:ident (&self$(, $fn_arg:ident: $fn_arg_type:ty)*)$(-> $fn_ret:ty)? {$($fn_stmt:stmt;)*})*
        $(fnmut $fn_mut:ident (&mut self$(, $fn_mut_arg:ident: $fn_mut_arg_type:ty)*)$(-> $fn_mut_ret:ty)? {$($fn_mut_stmt:stmt;)*})*
    }) => {
        let generated_impl = $code.new_impl($object)$(.impl_trait($trait))?;
        $(generated_impl.associate_const(stringify!($const), stringify!($const_type), $const_val, "");)*
        $(generated_impl.associate_type(stringify!($type), $type_val);)*
        $(generated_impl.new_fn(stringify!($get)).ret(stringify!($get_ret)).line($get_value);)*
        $(generated_impl.new_fn(stringify!($fn))
            .arg_ref_self()
            $(.arg(stringify!($fn_arg), stringify!($fn_arg_type)))*
            $(.ret(stringify!($fn_ret)))?
            $(.line(concat!(stringify!($fn_stmt), ";")))*
            ;
        )*
        $(generated_impl.new_fn(stringify!($fn_mut))
            .arg_mut_self()
            $(.arg(stringify!($fn_mut_arg), stringify!($fn_mut_arg_type)))*
            $(.ret(stringify!($fn_mut_ret)))?
            $(.line(concat!(stringify!($fn_mut_stmt), ";")))*
            ;
        )*
    };
}

pub use derive_rust_object;
pub use generate_impl;
