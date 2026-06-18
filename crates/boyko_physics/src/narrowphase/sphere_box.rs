//! Sphere-vs-OBB contact generation (P2 W4).
//!
//! Single-point contact: transform the sphere center into the box's local frame,
//! clamp to the box half-extents (the closest point on / in the OBB), and read
//! the contact off that. ZERO `unsafe`, no allocation, deterministic.

use crate::manifold::{BodyIndex, ContactPoint, Manifold};
use crate::math::{Quat, Vec3};

use super::feature_vertex_face;

/// Generates the sphere-box contact between sphere body `a` and box body `b`, or
/// `None` when they do not overlap (P2 W4).
///
/// `sphere_center` / `sphere_radius` describe the sphere (body A); `box_center` /
/// `box_rotation` / `box_half` describe the OBB (body B). The emitted `normal`
/// points from A toward B (the sphere-sphere convention): it is the direction
/// from the sphere center toward the closest box point, NEGATED so it runs A→B.
///
/// The closest point on the OBB to the sphere center is found by transforming the
/// center into the box's local frame, clamping each local coordinate to the box
/// half-extent, and transforming back. When the sphere center is OUTSIDE the box,
/// `dist = |center − closest|` and the contact exists iff `dist < sphere_radius`.
/// When the center is INSIDE the box (`closest == center` in local frame), the
/// normal is the least-penetrated local face axis (the standard sphere-inside-box
/// fallback), and the separation is `−(face_distance + sphere_radius)`.
///
/// Exactly one contact point with `feature_id = feature_vertex_face(0)` (a single
/// point has no incident vertex to disambiguate; the class tag still keeps it
/// disjoint from face-face / edge-edge ids).
#[allow(clippy::too_many_arguments)]
pub fn sphere_box_contact(
    body_a: BodyIndex,
    body_b: BodyIndex,
    sphere_center: Vec3,
    sphere_radius: f32,
    box_center: Vec3,
    box_rotation: Quat,
    box_half: Vec3,
) -> Option<Manifold> {
    // Sphere center expressed in the box's LOCAL frame (axis-aligned there).
    let local = box_rotation.inverse_rotate(sphere_center - box_center);

    // Closest point on the box (clamped to the half-extents, per local axis).
    let clamped = local.clamp_symmetric(box_half);
    let inside = clamped == local;

    let (local_normal, separation) = if inside {
        // Sphere center is INSIDE the box: push out along the least-penetrated
        // local face axis (the axis where the center is closest to a face).
        let dx = box_half.x - local.x.abs();
        let dy = box_half.y - local.y.abs();
        let dz = box_half.z - local.z.abs();
        // Pick the minimum face distance; the normal is that axis, signed toward
        // the nearer face (matching the center's sign on that axis).
        let (axis_dist, axis_normal) = if dx <= dy && dx <= dz {
            (dx, Vec3::new(local.x.signum(), 0.0, 0.0))
        } else if dy <= dz {
            (dy, Vec3::new(0.0, local.y.signum(), 0.0))
        } else {
            (dz, Vec3::new(0.0, 0.0, local.z.signum()))
        };
        // Deepest overlap: the sphere fully overlaps the face by axis_dist plus
        // its own radius. Separation is negative (penetrating).
        (axis_normal, -(axis_dist + sphere_radius))
    } else {
        // Sphere center is OUTSIDE the box: the closest point is on the surface.
        let offset = local - clamped;
        let dist = offset.length();
        if dist >= sphere_radius {
            // No overlap.
            return None;
        }
        let n = if dist > f32::MIN_POSITIVE {
            offset * dist.recip()
        } else {
            // Center exactly on the surface: fall back to the +x local face.
            Vec3::new(1.0, 0.0, 0.0)
        };
        (n, dist - sphere_radius)
    };

    // `local_normal` points from the box surface toward the sphere center (away
    // from B, toward A) in the box's local frame. Rotate to world, then NEGATE so
    // the manifold normal runs A (sphere) → B (box).
    let world_normal = box_rotation.rotate(local_normal);
    let normal = world_normal * -1.0;

    // World closest point on the box surface (the contact anchor on B).
    let world_closest = box_center + box_rotation.rotate(clamped);
    // Anchor on A: the sphere-surface point along the contact normal from the
    // sphere center toward the box.
    let anchor_a = sphere_center + normal * sphere_radius;

    let mut manifold = Manifold::new(body_a, body_b);
    manifold.normal = normal;
    manifold.points[0] = ContactPoint {
        anchor_a,
        anchor_b: world_closest,
        separation,
        feature_id: feature_vertex_face(0),
    };
    manifold.count = 1;
    Some(manifold)
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: BodyIndex = BodyIndex(0);
    const B: BodyIndex = BodyIndex(1);

    /// A sphere resting on the +y face of an axis-aligned unit box: the contact
    /// normal runs A→B (downward, −y), separation is the small overlap, and the
    /// box anchor is on its top face.
    #[test]
    fn sphere_on_box_top_face() {
        let box_half = Vec3::new(1.0, 1.0, 1.0);
        // Sphere of radius 0.5 centered at y = 1.4: its bottom (y = 0.9) is 0.1
        // below the box top (y = 1.0), so they overlap by 0.1.
        let m = sphere_box_contact(
            A,
            B,
            Vec3::new(0.0, 1.4, 0.0),
            0.5,
            Vec3::ZERO,
            Quat::IDENTITY,
            box_half,
        )
        .expect("overlapping sphere-box must contact");
        assert_eq!(m.count, 1);
        // Normal A→B points DOWN (sphere above box).
        assert!(m.normal.y < -0.99, "normal must run sphere→box (−y): {:?}", m.normal);
        assert!((m.points[0].separation - (-0.1)).abs() < 1e-5, "sep {}", m.points[0].separation);
        // Box anchor on its top face (y = +1).
        assert!((m.points[0].anchor_b.y - 1.0).abs() < 1e-5, "anchor {:?}", m.points[0].anchor_b);
    }

    /// A sphere well clear of the box generates no contact.
    #[test]
    fn sphere_clear_of_box_is_none() {
        let m = sphere_box_contact(
            A,
            B,
            Vec3::new(0.0, 5.0, 0.0),
            0.5,
            Vec3::ZERO,
            Quat::IDENTITY,
            Vec3::new(1.0, 1.0, 1.0),
        );
        assert!(m.is_none(), "a distant sphere must not contact the box");
    }

    /// A sphere touching the +x face of a box ROTATED 90° about z: the closest
    /// point and normal must be computed in the box's local frame (an AABB test
    /// would get this wrong).
    #[test]
    fn sphere_against_rotated_box_uses_local_frame() {
        // 90° about +z swaps local x↔y. A box with local half (2, 0.5, 0.5)
        // rotated 90°-z presents its long axis along world y. A sphere placed
        // along world +y near the rotated long face must still find a contact.
        let half = std::f32::consts::FRAC_PI_4; // θ/2 for 90°
        let rot = Quat::new(0.0, 0.0, half.sin(), half.cos());
        let box_half = Vec3::new(2.0, 0.5, 0.5);
        // After 90°-z, local +x maps to world +y; the box extends to world y = 2.
        // Place a sphere just above world y = 2 so it overlaps the long face.
        let m = sphere_box_contact(
            A,
            B,
            Vec3::new(0.0, 2.4, 0.0),
            0.5,
            Vec3::ZERO,
            rot,
            box_half,
        )
        .expect("rotated long face must contact");
        assert_eq!(m.count, 1);
        // Contact normal runs sphere→box ≈ −y (down onto the long face).
        assert!(m.normal.y < -0.9, "rotated-box normal must run −y: {:?}", m.normal);
    }

    /// A sphere whose center is INSIDE the box pushes out along the least-
    /// penetrated face (the inside fallback), with a deeply negative separation.
    #[test]
    fn sphere_center_inside_box_pushes_out_least_axis() {
        let box_half = Vec3::new(2.0, 1.0, 3.0);
        // Center near the +y face from inside (least distance is the y axis).
        let m = sphere_box_contact(
            A,
            B,
            Vec3::new(0.0, 0.8, 0.0),
            0.5,
            Vec3::ZERO,
            Quat::IDENTITY,
            box_half,
        )
        .expect("a center inside the box always contacts");
        // Least-penetrated axis is +y (0.2 to the face vs 2.0 / 3.0 elsewhere),
        // so the local push is +y → A→B normal is −y.
        assert!(m.normal.y < -0.99, "inside fallback normal: {:?}", m.normal);
        // Deeply penetrating: separation = −(face_dist + radius) = −(0.2 + 0.5).
        assert!((m.points[0].separation - (-0.7)).abs() < 1e-5, "sep {}", m.points[0].separation);
    }
}
