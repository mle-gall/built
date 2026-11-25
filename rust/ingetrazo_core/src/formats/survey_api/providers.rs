//! Map and elevation providers, the cache layout and window-owned service callbacks.
use super::*;

pub(super) fn tiles(api: &Api<'_>) -> PyResult<()> {
    api.functions(&[
        "deg2num",
        "num2deg",
        "tile_bbox",
        "tiles_covering",
        "TileCache",
        "default_cache_dir",
        "custom_source",
        "source_slug",
    ])?;
    api.record(
        "TileSource",
        &[
            Field("id", "str", Required),
            Field("name", "str", Required),
            Field("url_template", "str", Required),
            Field("max_zoom", "int", Int(19)),
            Field("attribution", "str", Text("")),
            Field("subdomains", "tuple[str, ...]", Factory("tuple")),
            Field("tile_size", "int", Int(256)),
        ],
        &api.methods(&[("url", "tile_url")], &[])?,
        true,
        true,
        &[],
    )?;
    let presets = PyDict::new(api.py());
    let source = api.module.getattr("TileSource")?;
    for (id,name,template,zoom,attribution) in [
        ("esri_imagery","Esri World Imagery (satellite)","https://server.arcgisonline.com/ArcGIS/rest/services/World_Imagery/MapServer/tile/{z}/{y}/{x}",19,"Esri, Maxar, Earthstar Geographics, and the GIS community"),
        ("s2cloudless","Sentinel-2 cloudless (EOX)","https://tiles.maps.eox.at/wmts/1.0.0/s2cloudless-2020_3857/default/GoogleMapsCompatible/{z}/{y}/{x}.jpg",17,"Sentinel-2 cloudless — https://s2maps.eu by EOX IT Services GmbH (Contains modified Copernicus Sentinel data)"),
        ("osm","OpenStreetMap (streets)","https://tile.openstreetmap.org/{z}/{x}/{y}.png",19,"© OpenStreetMap contributors"),
    ]{presets.set_item(id,source.call1((id,name,template,zoom,attribution))?)?;}
    api.module.add("PRESETS", presets)?;
    api.module.add("DEFAULT_SOURCE_ID", "esri_imagery")?;
    let members = api.methods(
        &[
            ("__init__", "tile_init"),
            ("to_dict", "tile_to_dict"),
            ("set_rectangle", "set_rectangle"),
            ("add_patch", "add_patch"),
            ("capture_bounds", "capture_bounds"),
            ("cap_detail", "cap_detail"),
            ("_patch_tiles", "patch_tiles"),
            ("flat_tiles", "flat_tiles"),
            ("visible_tiles", "visible_tiles"),
            ("quad_local", "quad_local"),
        ],
        &[],
    )?;
    members.set_item(
        "from_dict",
        api.py()
            .import("builtins")?
            .call_method1("classmethod", (api.core.getattr("tile_from_dict")?,))?,
    )?;
    api.class("TileLayer", None, &members)?;
    Ok(())
}

pub(super) fn transport(api: &Api<'_>) -> PyResult<()> {
    api.exports(&[
        ("_download", "survey_download"),
        ("request", "survey_request"),
    ])?;
    let base = api.core.getattr("SurveyService")?;
    let members = PyDict::new(api.py());
    members.set_item("__init__", base.getattr("bind_host")?)?;
    api.class("Service", Some(base), &members)?;
    Ok(())
}
fn service<'p>(
    api: &Api<'p>,
    name: &str,
    members: &Bound<'p, PyDict>,
    signals: &[&str],
) -> PyResult<()> {
    for signal in signals {
        members.set_item(*signal, api.core.call_method0("SurveySignal")?)?;
    }
    api.class(
        name,
        Some(api.py().import("ingetrazo_core.georef.transport")?.getattr("Service")?),
        members,
    )?;
    Ok(())
}
pub(super) fn tile_fetcher(api: &Api<'_>) -> PyResult<()> {
    api.functions(&["TileCache", "default_cache_dir"])?;
    let members = api.methods(
        &[
            ("__init__", "fetch_init"),
            ("request", "tile_request"),
            ("_pump", "tile_pump"),
            ("_start_download", "tile_start"),
            ("_completed", "tile_completed"),
            ("cancel_all", "tile_cancel"),
        ],
        &[],
    )?;
    members.set_item("_MAX_INFLIGHT", 8)?;
    let py = api.py();
    let getter = py
        .import("operator")?
        .call_method1("attrgetter", ("_cache",))?;
    members.set_item(
        "cache",
        py.import("builtins")?.call_method1("property", (getter,))?,
    )?;
    service(api, "TileFetcher", &members, &["tileReady", "tileFailed"])
}
pub(super) fn dem(api: &Api<'_>) -> PyResult<()> {
    api.functions(&["decode_elevation", "bilinear", "pixel_in_tile"])?;
    api.module.add("TILE_PX", 256)?;
    api.record(
        "DEMSource",
        &[
            Field("id", "str", Required),
            Field("name", "str", Required),
            Field("url_template", "str", Required),
            Field("encoding", "str", Text("terrarium")),
            Field("max_zoom", "int", Int(15)),
            Field("attribution", "str", Text("")),
        ],
        &api.methods(&[("url", "tile_url")], &[])?,
        true,
        true,
        &[],
    )?;
    api.module.add("AWS_TERRAIN",api.module.call_method1("DEMSource",("aws_terrarium","AWS Terrain Tiles (SRTM/NED)","https://s3.amazonaws.com/elevation-tiles-prod/terrarium/{z}/{x}/{y}.png","terrarium",15,"Terrain Tiles hosted on AWS by Mapzen/Nextzen — sources include SRTM, NED, Copernicus. https://registry.opendata.aws/terrain-tiles/"))?)?;
    service(
        api,
        "DEMSampler",
        &api.methods(
            &[
                ("__init__", "dem_init"),
                ("_ensure_fetcher", "dem_fetcher"),
                ("_on_tile", "dem_tile"),
                ("_field_for", "dem_field"),
                ("ensure_area", "dem_area"),
                ("elevation_at", "dem_elevation"),
                ("elevation_at_local", "dem_local"),
                ("has_tile", "dem_has"),
            ],
            &[],
        )?,
        &["changed"],
    )
}
pub(super) fn geocode(api: &Api<'_>) -> PyResult<()> {
    api.functions(&["geocode_url", "parse_places", "parse_ip"])?;
    api.module
        .add("NOMINATIM", "https://nominatim.openstreetmap.org/search")?;
    api.module.add("IPAPI", "https://ipapi.co/json/")?;
    api.record(
        "Place",
        &[
            Field("name", "str", Required),
            Field("lat", "float", Required),
            Field("lon", "float", Required),
        ],
        &PyDict::new(api.py()),
        true,
        true,
        &[],
    )?;
    service(
        api,
        "Geocoder",
        &api.methods(
            &[
                ("__init__", "geocoder_init"),
                ("_get", "geo_get"),
                ("search", "geo_search"),
                ("_received", "geocode_received"),
            ],
            &[],
        )?,
        &["resultsReady", "failed"],
    )?;
    service(
        api,
        "IpLocator",
        &api.methods(
            &[
                ("__init__", "geocoder_init"),
                ("_get", "geo_get"),
                ("locate", "geo_locate"),
                ("_received", "ip_received"),
            ],
            &[],
        )?,
        &["located", "failed"],
    )
}
