use crate::{game::level_objects::PlaneDesc, net::EntityNetId};
use serde::{Deserialize, Serialize};

#[derive(Default)]
pub struct LevelState {
    pub objects: Vec<LevelObject>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct LevelObject {
    pub net_id: EntityNetId,
    pub desc: LevelObjectDesc,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum LevelObjectDesc {
    Plane(PlaneDesc),
}
