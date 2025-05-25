use tokio::fs;
use tokio::sync::OnceCell;

async fn read_resource(resource_path: &str) -> Vec<u8> {
    fs::read(resource_path).await
        .expect("Could not load resources")
}

async fn read_js_with_endpoint(ws_endpoint: &str) -> Vec<u8> {
    let javascript = fs::read_to_string("src/static/app.js").await;
        javascript
        .expect("Could not load javascript")
        .replace("SOCKET_HOST", ws_endpoint).into_bytes()
}

pub struct StaticResource {
    pub homepage: Vec<u8>,
    pub javascript: Vec<u8>,
    pub empty_cell: Vec<u8>,
    pub x_cell: Vec<u8>,
    pub o_cell: Vec<u8>,
    pub css: Vec<u8>,
    pub favicon: Vec<u8>,
}

impl StaticResource {
    pub async fn new(ws_endpoint: &str) -> &'static StaticResource {
        STATIC_RESOURCE.get_or_init(|| async {
            let resources = tokio::join!(
                read_resource("src/static/index.html"),
                read_js_with_endpoint(ws_endpoint),
                read_resource("src/static/images/empty-cell.jpg"),
                read_resource("src/static/images/x-cell.jpg"),
                read_resource("src/static/images/o-cell.jpg"),
                read_resource("src/static/grid.css"),
                read_resource("src/static/images/favicon.png"),
            );
            StaticResource {
                homepage: resources.0,
                javascript: resources.1,
                empty_cell: resources.2,
                x_cell: resources.3,
                o_cell: resources.4,
                css: resources.5,
                favicon: resources.6,
            }
        }).await
    }
}

static STATIC_RESOURCE: OnceCell<StaticResource> = OnceCell::const_new();