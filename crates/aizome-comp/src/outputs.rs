//! Outputs in the global layout: adding them, changing their scale,
//! position and transform, and describing them over IPC.

use aizome_ipc::{Event, Mode as IpcMode, OutputInfo, Rect as IpcRect, Transform as IpcTransform};
use smithay::desktop::layer_map_for_output;
use smithay::output::{Mode, Output, Scale};
use smithay::utils::Transform;

use crate::state::{Aizome, OutputState};

/// A requested change to an output (from IPC, the settings file or
/// wlr-output-management).
#[derive(Clone, Debug, Default)]
pub struct OutputChange {
    pub name: String,
    /// Width, height, refresh (mHz).
    pub mode: Option<(i32, i32, Option<i32>)>,
    pub scale: Option<f64>,
    pub position: Option<(i32, i32)>,
    pub transform: Option<Transform>,
    pub enabled: Option<bool>,
}

/// smithay's scale for a number: integer where it is one.
pub fn scale_of(s: f64) -> Scale {
    if (s - s.round()).abs() < 1e-6 { Scale::Integer(s.round() as i32) } else { Scale::Fractional(s) }
}

/// Puts an output into the layout and advertises it.
pub fn add_output(az: &mut Aizome, output: &Output, mode: Mode, scale: f64, position: (i32, i32), _serial: &str) {
    let _global = output.create_global::<crate::state::State>(&az.dh);
    output.change_current_state(Some(mode), Some(Transform::Normal), Some(scale_of(scale)), Some(position.into()));
    az.outputs.push(output.clone());
    az.out.insert(output.clone(), OutputState::new(mode.refresh));
    layer_map_for_output(output).arrange();
    az.window_outputs_dirty = true;
    az.queue_redraw(output);
    outputs_changed(az);
}

/// Takes an output out of the layout (unplugged or disabled).
pub fn remove_output(az: &mut Aizome, output: &Output) {
    az.outputs.retain(|o| o != output);
    az.out.remove(output);
    let mut map = layer_map_for_output(output);
    let layers: Vec<_> = map.layers().cloned().collect();
    for l in layers {
        l.layer_surface().send_close();
        map.unmap_layer(&l);
    }
    drop(map);
    az.window_outputs_dirty = true;
    crate::actions::rescue_windows(az);
    outputs_changed(az);
}

/// Applies scale, position and transform (the parts every backend shares).
pub fn apply_logical(az: &mut Aizome, change: &OutputChange) -> Result<(), String> {
    let output = az.output_by_name(&change.name).ok_or_else(|| format!("no output {}", change.name))?;
    let scale = change.scale.map(|s| {
        if !(0.5..=4.0).contains(&s) {
            return Err(format!("scale {s} is out of range"));
        }
        Ok(scale_of(s))
    });
    let scale = match scale {
        Some(Ok(s)) => Some(s),
        Some(Err(e)) => return Err(e),
        None => None,
    };
    output.change_current_state(None, change.transform, scale, change.position.map(|p| p.into()));
    layer_map_for_output(&output).arrange();
    az.window_outputs_dirty = true;
    az.queue_redraw_all();
    outputs_changed(az);
    Ok(())
}

pub fn transform_to_ipc(t: Transform) -> IpcTransform {
    match t {
        Transform::Normal => IpcTransform::Normal,
        Transform::_90 => IpcTransform::_90,
        Transform::_180 => IpcTransform::_180,
        Transform::_270 => IpcTransform::_270,
        Transform::Flipped => IpcTransform::Flipped,
        Transform::Flipped90 => IpcTransform::Flipped90,
        Transform::Flipped180 => IpcTransform::Flipped180,
        Transform::Flipped270 => IpcTransform::Flipped270,
    }
}

pub fn transform_from_ipc(t: IpcTransform) -> Transform {
    match t {
        IpcTransform::Normal => Transform::Normal,
        IpcTransform::_90 => Transform::_90,
        IpcTransform::_180 => Transform::_180,
        IpcTransform::_270 => Transform::_270,
        IpcTransform::Flipped => Transform::Flipped,
        IpcTransform::Flipped90 => Transform::Flipped90,
        IpcTransform::Flipped180 => Transform::Flipped180,
        IpcTransform::Flipped270 => Transform::Flipped270,
    }
}

pub fn transform_from_config(t: aizome_config::Transform) -> Transform {
    use aizome_config::Transform as C;
    match t {
        C::Normal => Transform::Normal,
        C::_90 => Transform::_90,
        C::_180 => Transform::_180,
        C::_270 => Transform::_270,
        C::Flipped => Transform::Flipped,
        C::Flipped90 => Transform::Flipped90,
        C::Flipped180 => Transform::Flipped180,
        C::Flipped270 => Transform::Flipped270,
    }
}

fn ipc_rect(r: crate::wm::Rect) -> IpcRect {
    IpcRect::new(r.x, r.y, r.w, r.h)
}

pub fn info(az: &Aizome, output: &Output) -> OutputInfo {
    let props = output.physical_properties();
    let mode = output.current_mode();
    let preferred = output.preferred_mode();
    let modes = output
        .modes()
        .into_iter()
        .map(|m| IpcMode { width: m.size.w, height: m.size.h, refresh: m.refresh, preferred: Some(m) == preferred })
        .collect();
    let serial = output.user_data().get::<OutputSerial>().map(|s| s.0.clone()).unwrap_or_default();
    OutputInfo {
        name: output.name(),
        make: props.make,
        model: props.model,
        serial,
        enabled: true,
        current_mode: mode.map(|m| IpcMode { width: m.size.w, height: m.size.h, refresh: m.refresh, preferred: Some(m) == preferred }),
        modes,
        scale: output.current_scale().fractional_scale(),
        transform: transform_to_ipc(output.current_transform()),
        logical: ipc_rect(az.output_rect(output)),
        usable: ipc_rect(az.usable_area(output)),
        physical_size_mm: (props.size.w, props.size.h),
        powered: az.out.get(output).is_none_or(|o| o.powered),
    }
}

/// An output's serial number, kept in its user data.
pub struct OutputSerial(pub String);

pub fn all_info(az: &Aizome) -> Vec<OutputInfo> {
    az.outputs.iter().map(|o| info(az, o)).collect()
}

pub fn outputs_changed(az: &mut Aizome) {
    let outputs = all_info(az);
    az.ipc.emit(&Event::OutputsChanged { outputs });
    crate::protocols::output_management::outputs_changed(az);
}
