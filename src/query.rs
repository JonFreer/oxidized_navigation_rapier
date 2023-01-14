
//! Module for querying the nav-mesh
use std::sync::{Arc, RwLock};

use bevy::prelude::{Vec3, UVec2};

use crate::{tiles::{NavMesh, Link, get_closest_point_in_polygon}, NavMeshSettings};

const HEURISTIC_SCALE: f32 = 0.999;

bitflags::bitflags! {
    #[derive(Default)]
    struct NodeFlags: u8 {
        const OPEN = 0b00000001;
        const CLOSED = 0b00000010;
    }
}

#[derive(Debug)]
struct NavMeshNode {
    position: Vec3,
    cost: f32,
    total_cost: f32,
    tile: UVec2,
    polygon: u16,
    flags: NodeFlags,
    parent: Option<usize>
}

/// Errors returned by [find_path]
pub enum FindPathError {
    /// Nav-mesh couldn't be retrieved from lock.
    NavMeshUnavailable,
    /// No polygon found near ``start_pos``. 
    NoValidStartPolygon,
    /// No polygon found near ``end_pos``.
    NoValidEndPolygon,
}

/// Performs A* pathfinding on the supplied nav-mesh.
/// Returning the polygons crossed as a Vec<(Tile, Polygon)> or [FindPathError]
/// 
/// * ``nav_mesh`` - Nav-mesh to pathfind across.
/// * ``nav_mesh_settings`` - Nav-mesh settings used to generate ``nav_mesh``.
/// * ``start_pos`` - Starting position for the path.
/// * ``end_pos`` - Destination position for the path, i.e where you want to go.
/// * ``position_search_radius`` - Radius to search for a start & end polygon in. In world units. If **``None``** is supplied a default value of ``5.0`` is used.
pub fn find_path(
    nav_mesh: Arc<RwLock<NavMesh>>,
    nav_mesh_settings: NavMeshSettings,
    start_pos: Vec3,
    end_pos: Vec3,
    position_search_radius: Option<f32>
) -> Result<Vec<(UVec2, u16)>, FindPathError> {
    let Ok(nav_mesh) = nav_mesh.read() else {
        return Err(FindPathError::NavMeshUnavailable);
    };

    let search_radius = position_search_radius.unwrap_or(5.0);

    let Some((start_tile, start_poly, start_pos)) = nav_mesh.find_closest_polygon_in_box(&nav_mesh_settings, start_pos, search_radius) else {
        return Err(FindPathError::NoValidStartPolygon);
    };

    let Some((end_tile, end_poly, end_pos)) = nav_mesh.find_closest_polygon_in_box(&nav_mesh_settings, end_pos, search_radius) else {
        return Err(FindPathError::NoValidEndPolygon);
    };

    if start_tile == end_tile && start_poly == end_poly {
        return Ok(vec![(start_tile, start_poly)]);
    }

    let mut nodes = Vec::with_capacity(30);
    let mut open_list = Vec::with_capacity(8);

    {
        let start_node = NavMeshNode {
            position: start_pos,
            cost: 0.0,
            total_cost: start_pos.distance(end_pos) * HEURISTIC_SCALE,
            tile: start_tile,
            polygon: start_poly,
            flags: NodeFlags::OPEN,
            parent: None,
        };
        
        nodes.push(start_node);
        open_list.push(0);
    }

    let mut last_best_node = 0;
    let mut last_best_node_cost = nodes[0].total_cost;
    
    while let Some(best_node_index) = open_list.pop() {
        let (best_tile, best_polygon, best_position, best_cost, best_parent) ={
            let node = &mut nodes[best_node_index];
            node.flags.remove(NodeFlags::OPEN);
            node.flags.insert(NodeFlags::CLOSED);

            if node.tile == end_tile && node.polygon == end_poly {
                last_best_node = best_node_index;
                break;
            }
            
            (node.tile, node.polygon, node.position, node.cost, node.parent)
        };

        let node_tile = nav_mesh.tiles.get(&best_tile).unwrap();

        for link in node_tile.polygons[best_polygon as usize].links.iter() {
            let (link_tile, link_polygon) = match link {
                Link::Internal { neighbour_polygon, .. } => (best_tile, *neighbour_polygon),
                Link::OffMesh { neighbour_polygon, direction, .. } => (direction.offset(best_tile), *neighbour_polygon),
            };

            // Don't go back to our parent.
            if let Some(parent) = best_parent {
                if nodes[parent].tile == link_tile && nodes[parent].polygon == link_polygon {
                    continue;
                }
            }

            let neighbour_node_index = if let Some(index) = nodes.iter().position(|element| element.tile == link_tile && element.polygon == link_polygon)  {
                index
            } else {
                // Node hasn't been visited already, let's create it.
                let position = match link {
                    Link::Internal { edge, .. } => {
                        // Just the midpoint of the current edge.
                        let indices = &node_tile.polygons[best_polygon as usize].indices;
                        let a = node_tile.vertices[indices[*edge as usize] as usize];
                        let b = node_tile.vertices[indices[(*edge + 1) as usize % indices.len()] as usize];

                        a.lerp(b, 0.5)
                    },
                    Link::OffMesh { edge, bound_min, bound_max, .. } => {
                        // The mid point of the current-edge sliced by bound_min & bound_max.
                        let indices = &node_tile.polygons[best_polygon as usize].indices;
                        let a = node_tile.vertices[indices[*edge as usize] as usize];
                        let b = node_tile.vertices[indices[(*edge + 1) as usize % indices.len()] as usize];
                        
                        const S: f32 = 1.0/255.0;
                        let bound_min = *bound_min as f32 * S;
                        let bound_max = *bound_max as f32 * S;
                        let clamped_a = a.lerp(b, bound_min);
                        let clamped_b = a.lerp(b, bound_max);

                        clamped_a.lerp(clamped_b, 0.5)
                    },
                };

                nodes.push(NavMeshNode {
                    position,
                    cost: 0.0,
                    total_cost: 0.0,
                    tile: link_tile,
                    polygon: link_polygon,
                    flags: NodeFlags::empty(),
                    parent: None,
                });

                nodes.len() - 1
            };

            let (old_flags, total_cost) = {
                let neighbour_node = &mut nodes[neighbour_node_index];

                // TODO: Ideally you want to be able to override this but for now we just go with the distance.
                let (cost, heuristic) = if end_tile == link_tile && end_poly == link_polygon { // Special case for the final node.
                    let current_cost = best_position.distance(neighbour_node.position);
                    let end_cost = neighbour_node.position.distance(end_pos);

                    let cost = best_cost + current_cost + end_cost;

                    (cost, 0.0)
                } else {
                    let current_cost = best_position.distance(neighbour_node.position);

                    let cost = best_cost + current_cost;
                    let heuristic = neighbour_node.position.distance(end_pos) * HEURISTIC_SCALE;

                    (cost, heuristic)
                };
                let total_cost = cost + heuristic;

                if neighbour_node.flags.intersects(NodeFlags::OPEN | NodeFlags::CLOSED) && total_cost >= neighbour_node.total_cost {
                    continue;
                }

                let old_flags = neighbour_node.flags;
                neighbour_node.parent = Some(best_node_index);
                neighbour_node.flags.remove(NodeFlags::CLOSED);
                neighbour_node.flags.insert(NodeFlags::OPEN);
                neighbour_node.cost = cost;
                neighbour_node.total_cost = total_cost;

                if heuristic < last_best_node_cost {
                    last_best_node_cost = heuristic;
                    last_best_node = neighbour_node_index;
                }

                (old_flags, total_cost)
            };

            if old_flags.contains(NodeFlags::OPEN) {
                // Node already exists. Let's remove it.
                if let Some(existing_index) = open_list.iter().position(|node| *node == neighbour_node_index) {
                    open_list.remove(existing_index);
                }
            }

            // We want to insert the node into the list so that the next entry has a lower total.
            if let Some(index) = open_list.iter().position(|node_index| nodes[*node_index].total_cost < total_cost) {
                open_list.insert(index, neighbour_node_index);
            } else { // There is no entry with a lower total.
                open_list.push(neighbour_node_index);
            }
        }
    }

    // Is this worth it? :shrug: It saves a lot of memory allocations which I think is important. All locations should also be pretty hot in cache in the next loop.
    let path_count = {
        let mut count = 0;
        let mut parent = Some(last_best_node);
        while let Some(parent_index) = parent {
            count += 1;
            parent = nodes[parent_index].parent;
        }

        count
    };

    let mut path = Vec::with_capacity(path_count);

    let mut parent = Some(last_best_node);
    while let Some(parent_index) = parent {
        let node = &nodes[parent_index];

        path.push((node.tile, node.polygon));

        parent = node.parent;
    }

    path.reverse();

    Ok(path)
}

#[derive(Debug)]
pub enum StringPullingError {
    PathEmpty,
    NavMeshUnavailable,
    MissingStartTile,
    MissingEndTile,
    MissingNodeTile,
    NoLinkBetweenPathPoints
}

/// Performs "string pulling" on a path of polygons. Used to convert [find_path]'s result to a world space path.
/// 
/// Returns the path as Vec<Vec3> or [StringPullingError]
pub fn perform_string_pulling_on_path(
    nav_mesh: Arc<RwLock<NavMesh>>,
    start_pos: Vec3,
    end_pos: Vec3,
    path: &[(UVec2, u16)]
) -> Result<Vec<Vec3>, StringPullingError> {
    if path.is_empty() {
        return Err(StringPullingError::PathEmpty);
    }
    
    let Ok(nav_mesh) = nav_mesh.read() else {
        return Err(StringPullingError::NavMeshUnavailable);
    };

    let Some(start_tile) = nav_mesh.tiles.get(&path[0].0) else {
        return Err(StringPullingError::MissingStartTile);
    };
    let Some(end_tile) = nav_mesh.tiles.get(&path.last().unwrap().0) else {
        return Err(StringPullingError::MissingEndTile);
    };

    let start_pos = get_closest_point_in_polygon(start_tile, &start_tile.polygons[path[0].1 as usize], start_pos);
    let end_pos = get_closest_point_in_polygon(end_tile, &end_tile.polygons[path.last().unwrap().1 as usize], end_pos);

    let mut string_path = Vec::with_capacity(path.len() + 2);
    string_path.push(start_pos);

    if path.len() > 1 {
        let mut portal_apex = start_pos;
        let mut portal_left = start_pos;
        let mut portal_right = start_pos;

        let mut left_index = 0;
        let mut right_index = 0;
        
        let mut i = 0;
        while i < path.len() {
            let (left, right) = if let Some(next) = path.get(i + 1) {
                let current = &path[i];
                // Find link between this and next in path.
                let Some(node_tile) = nav_mesh.tiles.get(&current.0) else {
                    return Err(StringPullingError::MissingNodeTile);
                };
                let is_internal = current.0 == next.0;
                let Some(link) = node_tile.polygons[current.1 as usize].links.iter().find(|link| { // This is a mess :)))
                    match link {
                        Link::Internal { neighbour_polygon, .. } => is_internal && next.1 == *neighbour_polygon,
                        Link::OffMesh { neighbour_polygon, direction, .. } => direction.offset(current.0) == next.0 && next.1 == *neighbour_polygon,
                    }
                }) else {
                    return Err(StringPullingError::NoLinkBetweenPathPoints);
                };

                let indices = &node_tile.polygons[current.1 as usize].indices;
                match link {
                    Link::Internal { edge, .. } => {
                        let a = node_tile.vertices[indices[*edge as usize] as usize];
                        let b = node_tile.vertices[indices[(*edge + 1) as usize % indices.len()] as usize];

                        (a, b)
                    },
                    Link::OffMesh { edge, bound_min, bound_max, .. } => {
                        let a = node_tile.vertices[indices[*edge as usize] as usize];
                        let b = node_tile.vertices[indices[(*edge + 1) as usize % indices.len()] as usize];

                        const S: f32 = 1.0/255.0;
                        let clamped_a = a.lerp(b, *bound_min as f32 * S);
                        let clamped_b = a.lerp(b, *bound_max as f32 * S);

                        (clamped_a, clamped_b)
                    },
                }
            } else {
                (end_pos, end_pos)
            };

            // Right vertex.
            if triangle_area_2d(portal_apex, portal_right, right) <= 0.0 {
                if portal_apex.distance_squared(portal_right) < (1.0/16384.0) || triangle_area_2d(portal_apex, portal_left, right) > 0.0 {
                    portal_right = right;
                    right_index = i;
                } else {
                    portal_apex = portal_left;

                    if *string_path.last().unwrap() != portal_apex {
                        string_path.push(portal_apex);
                    }

                    portal_left = portal_apex;
                    portal_right = portal_apex;
                    right_index = left_index;

                    i = left_index + 1;
                    continue;
                }
            }

            // Left vertex.
            if triangle_area_2d(portal_apex, portal_left, left) >= 0.0 {
                if portal_apex.distance_squared(portal_left) < (1.0/16384.0)  || triangle_area_2d(portal_apex, portal_right, left) < 0.0 {
                    portal_left = left;
                    left_index = i;
                } else {
                    portal_apex = portal_right;

                    if *string_path.last().unwrap() != portal_apex {
                        string_path.push(portal_apex);
                    }

                    portal_left = portal_apex;
                    portal_right = portal_apex;
                    left_index = right_index;

                    i = right_index + 1;
                    continue;
                }
            }

            i += 1;
        }
    }

    string_path.push(end_pos);

    Ok(string_path)
}

fn triangle_area_2d(
    a: Vec3,
    b: Vec3,
    c: Vec3
) -> f32 {
    let ab_x = b.x - a.x;
    let ab_z = b.z - a.z;

    let ac_x = c.x - a.x;
    let ac_z = c.z - a.z;

    ac_x * ab_z - ab_x * ac_z
}