use {
    cargo_metadata::PackageId,
    serde::Serialize,
    std::{
        collections::{HashMap, HashSet},
        path::PathBuf,
    },
};

#[derive(Debug, Clone, Serialize)]
pub struct PackageInfo {
    pub name: String,
    pub path: PathBuf,
    pub dependencies: HashSet<PackageId>,
}

#[derive(Debug)]
pub struct PublishOrderData {
    pub levels: Vec<Vec<PackageId>>,
    pub id_to_level: HashMap<PackageId, usize>,
    pub id_to_package_info: HashMap<PackageId, PackageInfo>,
}
