use std::fs;
use tokio::sync::OnceCell;

fn read_resource(resource_path: &str) -> Vec<u8> {
    fs::read(resource_path).expect("Could not load resources")
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
    pub async fn new() -> &'static StaticResource {
        STATIC_RESOURCE.get_or_init(|| async {
            StaticResource {
                homepage: read_resource("src/static/index.html"),
                javascript: read_resource("src/static/app.js"),
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