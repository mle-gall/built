//! Metadata, geometry and sampling records exposed to survey scripts.
use super::*;

pub(super) fn geoimport(api: &Api<'_>) -> PyResult<()> {
    api.functions(&["load_features", "parse_geojson", "parse_kml", "parse_kmz"])?;
    api.record(
        "GeoFeature",
        &[
            Field("points", "list[tuple[float, float]]", Factory("list")),
            Field("closed", "bool", Bool(false)),
            Field("name", "str", Text("")),
        ],
        &api.methods(&[], &[])?,
        false,
        true,
        &[],
    )?;
    Ok(())
}

pub(super) fn geotiff(api: &Api<'_>) -> PyResult<()> {
    api.functions(&["unsupported_reason", "read_rgba", "describe"])?;
    api.exports(&[("read_info", "geotiff_info")])?;
    api.module.add("COMPRESSION_NONE", 1)?;
    api.module.add("COMPRESSION_LZW", 5)?;
    api.module.add("COMPRESSION_JPEG", 7)?;
    api.module.add("COMPRESSION_DEFLATE", 8)?;
    api.module.add("COMPRESSION_PACKBITS", 32773)?;
    api.module.add("COMPRESSION_DEFLATE_OLD", 32946)?;
    api.module
        .add("Path", api.py().import("pathlib")?.getattr("Path")?)?;
    api.module
        .add("Optional", api.py().import("typing")?.getattr("Optional")?)?;
    api.class(
        "GeoTiffError",
        Some(
            api.py()
                .get_type::<pyo3::exceptions::PyValueError>()
                .into_any(),
        ),
        &PyDict::new(api.py()),
    )?;
    api.record(
        "GeoTiffInfo",
        &[
            Field("path", "Path", Required),
            Field("width", "int", Required),
            Field("height", "int", Required),
            Field("samples", "int", Required),
            Field("bits", "tuple", Required),
            Field("compression", "int", Required),
            Field("photometric", "int", Required),
            Field("planar", "int", Required),
            Field("tiled", "bool", Required),
            Field("tile_w", "int", Required),
            Field("tile_h", "int", Required),
            Field("offsets", "tuple", Required),
            Field("bytecounts", "tuple", Required),
            Field("predictor", "int", Int(1)),
            Field("jpeg_tables", "bytes", EmptyBytes),
            Field("extra_samples", "tuple", EmptyTuple),
            Field("sample_format", "tuple", EmptyTuple),
            Field("big", "bool", Bool(false)),
            Field("byte_order", "str", Text("<")),
            Field("transform", "Optional[tuple]", Null),
            Field("epsg", "Optional[int]", Null),
            Field("zone", "Optional[int]", Null),
            Field("northern", "Optional[bool]", Null),
            Field("geographic", "bool", Bool(false)),
            Field("citation", "str", Text("")),
            Field("nodata", "Optional[str]", Null),
        ],
        &api.methods(
            &[
                ("pixel_to_map", "pixel_to_map"),
                ("corners_geodetic", "corners_geodetic"),
                ("ground_size", "ground_size"),
            ],
            &[("georeferenced", "georeferenced")],
        )?,
        false,
        true,
        &[],
    )?;
    Ok(())
}

pub(super) fn photomesh(api: &Api<'_>) -> PyResult<()> {
    api.functions(&[
        "atlas_bytes",
        "find_anchor",
        "load_atlas",
        "load_odm_obj",
        "parse_anchor",
        "parse_mtl",
        "plan_texture_sizes",
        "vertical_origin",
    ])?;
    api.exports(&[("pack_mesh", "photo_pack"), ("unpack_mesh", "photo_unpack")])?;
    api.module.add("VERTICAL_ODM", "odm")?;
    api.module.add("VERTICAL_DEM", "dem")?;
    api.module.add("VERTICAL_LOCAL", "local")?;
    api.module.add("GEOMETRY_ENTRY", "survey/geometry.npz")?;
    api.module.add("ATLAS_ENTRY", "survey/atlas-{:02d}.jpg")?;
    api.module.add("FOOT_PERCENTILE", 1.0)?;
    api.module.add("DEFAULT_TEXTURE_BUDGET", 1073741824)?;
    api.module
        .add("Path", api.py().import("pathlib")?.getattr("Path")?)?;
    api.record(
        "ODMAnchor",
        &[
            Field("east", "float", Required),
            Field("north", "float", Required),
            Field("zone", "int", Required),
            Field("northern", "bool", Required),
        ],
        &api.methods(&[], &[("hemisphere", "photo_hemisphere")])?,
        true,
        true,
        &[],
    )?;
    api.record(
        "PhotoMaterial",
        &[
            Field("name", "str", Required),
            Field("texture", "Path | None", Required),
            Field("start", "int", Required),
            Field("count", "int", Required),
        ],
        &api.methods(&[], &[])?,
        false,
        true,
        &[],
    )?;
    api.record(
        "PhotoMesh",
        &[
            Field("vertices", "object", Required),
            Field("uvs", "object", Required),
            Field("triangles", "object", Required),
            Field("materials", "list", Required),
            Field("source", "Path | None", Null),
            Field("anchor", "ODMAnchor | None", Null),
            Field("visible", "bool", Bool(true)),
            Field("images", "dict", Factory("dict")),
            Field("vertical_reference", "str", Text("odm")),
            Field("layer", "str | None", Null),
            Field("_index", "object", Null),
        ],
        &api.methods(
            &[
                ("invalidate_index", "photo_invalidate_index"),
                ("bounds", "photo_bounds"),
                ("height_at", "photo_height"),
                ("_ensure_index", "photo_index"),
            ],
            &[
                ("vertex_count", "photo_vertex_count"),
                ("triangle_count", "photo_triangle_count"),
                ("georeferenced", "photo_is_placed"),
                ("missing_textures", "photo_missing"),
            ],
        )?,
        false,
        false,
        &["visible", "images", "vertical_reference", "layer", "_index"],
    )?;
    api.record(
        "PhotoMeshSampler",
        &[
            Field("mesh", "PhotoMesh", Required),
            Field("datum", "object", Required),
        ],
        &api.methods(
            &[
                ("ensure_area", "photo_ensure_area"),
                ("elevation_at_local", "photo_sample"),
                ("elevation_at", "photo_geodetic"),
            ],
            &[],
        )?,
        false,
        false,
        &[],
    )?;
    Ok(())
}

pub(super) fn profile(api: &Api<'_>) -> PyResult<()> {
    api.functions(&[
        "selected_geopath",
        "sample_profile",
        "point_at_station",
        "polyline_length",
        "profile_to_csv",
    ])?;
    let notes = PyDict::new(api.py());
    notes.set_item("odm","photogrammetric survey (ODM); WGS84 ellipsoidal heights unless the processing used ground control points")?;
    notes.set_item("dem", "global DEM (SRTM-derived, orthometric)")?;
    notes.set_item(
        "local",
        "local reference: heights are relative, not absolute",
    )?;
    api.module.add("VERTICAL_NOTES", notes)?;
    api.record(
        "ProfileSample",
        &[
            Field("station", "float", Required),
            Field("x", "float", Required),
            Field("y", "float", Required),
            Field("elevation", "float | None", Required),
        ],
        &api.methods(&[], &[])?,
        false,
        true,
        &[],
    )?;
    api.record(
        "Profile",
        &[
            Field("samples", "list[ProfileSample]", Factory("list")),
            Field("length", "float", Float(0.0)),
            Field("complete", "bool", Bool(true)),
        ],
        &api.methods(
            &[
                ("elevations", "elevations"),
                ("min_elevation", "min_elevation"),
                ("max_elevation", "max_elevation"),
                ("total_gain", "total_gain"),
                ("slopes", "slopes"),
            ],
            &[],
        )?,
        false,
        true,
        &[],
    )?;
    Ok(())
}

pub(super) fn terrain(api: &Api<'_>) -> PyResult<()> {
    api.functions(&["build_mosaic", "build_terrain"])?;
    api.exports(&[("_bbox_ll", "terrain_geodetic_bounds")])?;
    api.module.add("TILE_PX", 256)?;
    api.record(
        "TerrainObject",
        &[
            Field("vertices", "list", Required),
            Field("uvs", "list", Required),
            Field("triangles", "list", Required),
            Field("tile_range", "tuple", Required),
            Field("nx", "int", Int(0)),
            Field("ny", "int", Int(0)),
            Field("bbox", "tuple", Box2d),
            Field("visible", "bool", Bool(true)),
            Field("texture_image", "object", Null),
        ],
        &api.methods(
            &[
                ("height_at", "terrain_height"),
                ("bounds", "terrain_bounds"),
            ],
            &[],
        )?,
        false,
        false,
        &["visible", "texture_image"],
    )?;
    Ok(())
}

pub(super) fn datum(api: &Api<'_>) -> PyResult<()> {
    api.functions(&["utm_forward", "utm_inverse", "zone_for_lon"])?;
    // Keep dataclasses.fields/asdict available on the public SceneDatum while
    // the coordinates and projection coefficients reside on the native base.
    let py = api.py();
    let dc = py.import("dataclasses")?;
    let fields = PyList::empty(py);
    for name in ["lat", "lon", "alt", "north"] {
        fields.append((
            name,
            py.get_type::<pyo3::types::PyFloat>(),
            dc.call_method0("field")?,
        ))?;
    }
    let kw = PyDict::new(py);
    kw.set_item("bases", (api.core.getattr("SceneDatum")?,))?;
    kw.set_item("init", false)?;
    let cls = dc.call_method("make_dataclass", ("SceneDatum", fields), Some(&kw))?;
    cls.setattr("__module__", "georef.datum")?;
    api.module.add("SceneDatum", cls)
}

fn tracked(
    api: &Api<'_>,
    name: &str,
    fields: &[&str],
    methods: &[(&str, &str)],
    restore: &str,
) -> PyResult<()> {
    let py = api.py();
    let doc = py.import("built_core.doc_model")?;
    let ns = api.methods(methods, &[])?;
    ns.set_item(
        "from_dict",
        py.import("builtins")?
            .call_method1("classmethod", (api.core.getattr(restore)?,))?,
    )?;
    let cls = api.class(name, Some(doc.getattr("Record")?), &ns)?;
    doc.call_method1("record_fields", PyTuple::new(py, fields)?)?
        .call1((&cls,))?;
    Ok(())
}
pub(super) fn points(api: &Api<'_>) -> PyResult<()> {
    api.functions(&["parse_points_csv", "points_from_rows", "datum_for_rows"])?;
    tracked(
        api,
        "GeoPoint",
        &["position", "name", "desc"],
        &[
            ("__init__", "geo_point_init"),
            ("to_dict", "geo_record_dict"),
        ],
        "geo_point_from",
    )
}
pub(super) fn geopath(api: &Api<'_>) -> PyResult<()> {
    tracked(
        api,
        "GeoPath",
        &["points", "closed", "name", "surface"],
        &[
            ("__init__", "geo_path_init"),
            ("segments", "path_segments"),
            ("length", "path_length"),
            ("perimeter", "path_length"),
            ("area", "path_area"),
            ("surface_area", "path_surface_area"),
            ("profile_points", "profile_points"),
            ("to_dict", "geo_record_dict"),
        ],
        "geo_path_from",
    )
}
