// use parry3d::{bounding_volume::Aabb, shape::TypedShape};
use rapier3d::geometry::Collider;

use super::OxidizedCollider;

/// This is only compiled and available when the "rapier" feature is enabled.
impl OxidizedCollider for Collider {
    fn oxidized_into_typed_shape(&self) -> rapier3d::prelude::TypedShape {
        self.shape().as_typed_shape()
        // self.raw.as_typed_shape()
    }

    fn oxidized_compute_local_aabb(&self) -> rapier3d::prelude::Aabb {
        self.shape().compute_local_aabb()
    }
}
