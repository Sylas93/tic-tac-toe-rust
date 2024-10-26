use std::fs;
use tokio::sync::OnceCell;

fn read_resource(resource_path: &str) -> Vec<u8> {
    fs::read(resource_path).expect("Could not load resources")
}

fn read_js_with_endpoint(ws_endpoint: &str) -> Vec<u8> {
    let javascript = fs::read_to_string("src/static/app.js")
        .expect("Could not load javascript");
    javascript.replace("SOCKET_HOST", ws_endpoint).into_bytes()
}

pub struct StaticResource {
    pub(crate) homepage: Vec<u8>,
    pub(crate) javascript: Vec<u8>,
    pub(crate) empty_cell: Vec<u8>,
    pub(crate) x_cell: Vec<u8>,
    pub(crate) o_cell: Vec<u8>,
    pub(crate) css: Vec<u8>,
    pub(crate) favicon: Vec<u8>,
}

impl StaticResource {
    pub async fn new(ws_endpoint: &str) -> &'static StaticResource {
        STATIC_RESOURCE.get_or_init(|| async {
            StaticResource {
                homepage: read_resource("src/static/index.html"),
                javascript: read_js_with_endpoint(ws_endpoint),
                empty_cell: read_resource("src/static/images/empty-cell.jpg"),
                x_cell: read_resource("src/static/images/x-cell.jpg"),
                o_cell: read_resource("src/static/images/o-cell.jpg"),
                css: read_resource("src/static/grid.css"),
                favicon: read_resource("src/static/images/favicon.png"),
            }
        }).await
    }
}

static STATIC_RESOURCE: OnceCell<StaticResource> = OnceCell::const_new();