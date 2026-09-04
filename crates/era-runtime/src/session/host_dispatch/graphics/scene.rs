#[allow(clippy::wildcard_imports)]
use super::*;

impl RuntimeSession {
    #[allow(clippy::too_many_lines)]
    pub(super) fn dispatch_scene_graphics(
        &mut self,
        vm: &mut RuntimeVm,
        request: &VmHostRequest,
        name: &str,
    ) -> Result<(), RuntimeError> {
        match name {
            "CBGCLEAR" => {
                let tokens = self.presentation.clear_client_backgrounds();
                self.remove_scene_interactions(tokens);
                commit_integer_result(vm, request.id, 1)?;
            }
            "CBGCLEARBUTTON" => {
                let tokens = self.presentation.clear_client_background_buttons();
                self.remove_scene_interactions(tokens);
                commit_integer_result(vm, request.id, 1)?;
            }
            "CBGREMOVEBMAP" => {
                self.presentation.clear_client_background_button_map();
                commit_integer_result(vm, request.id, 1)?;
            }
            "CBGREMOVERANGE" => {
                let minimum = integer_argument_value(request, 0)?;
                let maximum = integer_argument_value(request, 1)?;
                let tokens = self
                    .presentation
                    .clear_client_background_range(minimum, maximum);
                self.remove_scene_interactions(tokens);
                commit_integer_result(vm, request.id, 1)?;
            }
            "CBGSETBMAPG" => {
                let canvas_id = integer_argument_value(request, 0)?;
                let Some((_, _, revision)) = self
                    .project_snapshot
                    .as_ref()
                    .and_then(|project| project.resource_graph.canvas_observation(canvas_id))
                else {
                    return commit_integer_result(vm, request.id, 0);
                };
                let source = era_runtime_protocol::SceneSourceV1::Canvas {
                    canvas_id,
                    resource_revision: revision,
                };
                if !self
                    .project_snapshot
                    .as_mut()
                    .is_some_and(|project| project.resource_graph.retain_scene_source(&source))
                {
                    return commit_integer_result(vm, request.id, 0);
                }
                self.presentation.set_client_background_button_map(source);
                commit_integer_result(vm, request.id, 1)?;
            }
            "CBGSETG" => {
                let canvas_id = integer_argument_value(request, 0)?;
                let Some((_, _, revision)) = self
                    .project_snapshot
                    .as_ref()
                    .and_then(|project| project.resource_graph.canvas_observation(canvas_id))
                else {
                    return commit_integer_result(vm, request.id, 0);
                };
                let x = i32_argument_value(request, 1)?;
                let y = i32_argument_value(request, 2)?;
                let depth = cbg_depth(request, 3)?;
                let source = era_runtime_protocol::SceneSourceV1::Canvas {
                    canvas_id,
                    resource_revision: revision,
                };
                if !self
                    .project_snapshot
                    .as_mut()
                    .is_some_and(|project| project.resource_graph.retain_scene_source(&source))
                {
                    return commit_integer_result(vm, request.id, 0);
                }
                self.presentation.add_client_background(
                    source,
                    depth,
                    x,
                    y,
                    0,
                    0,
                    u8::MAX,
                    None,
                    None,
                );
                commit_integer_result(vm, request.id, 1)?;
            }
            "CBGSETSPRITE" => {
                let sprite_name = string_argument_value(request, 0, name)?.to_owned();
                let Some(revision) = self
                    .project_snapshot
                    .as_ref()
                    .and_then(|project| project.resource_graph.sprite_revision(&sprite_name))
                else {
                    return commit_integer_result(vm, request.id, 0);
                };
                let x = optional_i32_argument(request, 1, 0)?;
                let y = optional_i32_argument(request, 2, 0)?;
                let depth = optional_cbg_depth(request, 3, 1)?;
                let width = optional_i32_argument(request, 4, 0)?;
                let height = optional_i32_argument(request, 5, 0)?;
                let opacity = optional_opacity(request, 6, u8::MAX);
                let color_matrix = optional_color_matrix(vm, request, 7)?;
                let source = era_runtime_protocol::SceneSourceV1::Sprite {
                    sprite_name,
                    resource_revision: revision,
                };
                if !self
                    .project_snapshot
                    .as_mut()
                    .is_some_and(|project| project.resource_graph.retain_scene_source(&source))
                {
                    return commit_integer_result(vm, request.id, 0);
                }
                self.presentation.add_client_background(
                    source,
                    depth,
                    x,
                    y,
                    width,
                    height,
                    opacity,
                    color_matrix,
                    None,
                );
                commit_integer_result(vm, request.id, 1)?;
            }
            "CBGSETBUTTONSPRITE" => {
                let value = integer_argument_value(request, 0)?;
                if !(0..=0xff_ffff).contains(&value) {
                    return commit_integer_result(vm, request.id, 0);
                }
                let sprite_name = string_argument_value(request, 1, name)?.to_owned();
                let hover_name = string_argument_value(request, 2, name)?.to_owned();
                let Some(sprite_revision) = self
                    .project_snapshot
                    .as_ref()
                    .and_then(|project| project.resource_graph.sprite_revision(&sprite_name))
                else {
                    return commit_integer_result(vm, request.id, 1);
                };
                let hover_source = self
                    .project_snapshot
                    .as_ref()
                    .and_then(|project| project.resource_graph.sprite_revision(&hover_name))
                    .map(
                        |resource_revision| era_runtime_protocol::SceneSourceV1::Sprite {
                            sprite_name: hover_name,
                            resource_revision,
                        },
                    );
                let source = era_runtime_protocol::SceneSourceV1::Sprite {
                    sprite_name,
                    resource_revision: sprite_revision,
                };
                let x = i32_argument_value(request, 3)?;
                let y = i32_argument_value(request, 4)?;
                let depth = cbg_depth(request, 5)?;
                let title = request.argument(6).map(display_value);
                let mut roots = vec![source.clone()];
                roots.extend(hover_source.iter().cloned());
                let retained = self
                    .project_snapshot
                    .as_mut()
                    .is_some_and(|project| project.resource_graph.retain_scene_sources(&roots));
                if !retained {
                    return commit_integer_result(vm, request.id, 0);
                }
                let token = self.allocate_interaction();
                self.presentation.add_client_background(
                    source,
                    depth,
                    x,
                    y,
                    0,
                    0,
                    u8::MAX,
                    None,
                    Some((token, value, hover_source, title)),
                );
                self.command_intents.insert(token, VmValue::Integer(value));
                commit_integer_result(vm, request.id, 1)?;
            }
            "SETIMAGELAYER" | "SETIMAGELAYERL" => {
                let sprite_name = string_argument_value(request, 0, name)?.to_owned();
                let Some(resource_revision) = self
                    .project_snapshot
                    .as_ref()
                    .and_then(|project| project.resource_graph.sprite_revision(&sprite_name))
                else {
                    commit_completion(vm, request.id, VmHostCompletion::Ready(HostReady::empty()))?;
                    return Ok(());
                };
                let depth = integer_argument_value(request, 1)?;
                let x = optional_i32_argument(request, 2, 0)?;
                let y = optional_i32_argument(request, 3, 0)?;
                let width = optional_i32_argument(request, 4, 0)?;
                let height = optional_i32_argument(request, 5, 0)?;
                let opacity = optional_opacity(request, 6, u8::MAX);
                let color_matrix = optional_color_matrix(vm, request, 7)?;
                let line_relative = name == "SETIMAGELAYERL";
                let follow_content = line_relative
                    || request
                        .argument(8)
                        .is_some_and(|value| integer_value_or_zero(value) != 0);
                let anchor = if line_relative {
                    era_runtime_protocol::SceneAnchorV1::DisplayLine {
                        line_id: self.presentation.current_line_id(),
                    }
                } else {
                    era_runtime_protocol::SceneAnchorV1::Viewport
                };
                let source = era_runtime_protocol::SceneSourceV1::Sprite {
                    sprite_name,
                    resource_revision,
                };
                if !self
                    .project_snapshot
                    .as_mut()
                    .is_some_and(|project| project.resource_graph.retain_scene_source(&source))
                {
                    commit_completion(vm, request.id, VmHostCompletion::Ready(HostReady::empty()))?;
                    return Ok(());
                }
                self.presentation.add_image_layer(
                    source,
                    depth,
                    anchor,
                    x,
                    y,
                    width,
                    height,
                    opacity,
                    color_matrix,
                    follow_content,
                );
                commit_completion(vm, request.id, VmHostCompletion::Ready(HostReady::empty()))?;
            }
            "CLEARIMAGELAYER" => {
                self.presentation
                    .clear_image_layer(integer_argument_value(request, 0)?);
                commit_completion(vm, request.id, VmHostCompletion::Ready(HostReady::empty()))?;
            }
            "CLEARIMAGELAYER_ALL" => {
                self.presentation.clear_image_layers();
                commit_completion(vm, request.id, VmHostCompletion::Ready(HostReady::empty()))?;
            }
            "EXISTSIMAGELAYER" => {
                let exists = self
                    .presentation
                    .image_layer_exists(integer_argument_value(request, 0)?);
                return commit_integer_result(vm, request.id, i64::from(exists));
            }
            _ => unreachable!("scene graphics dispatch is exhaustive"),
        }
        self.emit_presentation()
    }

    fn remove_scene_interactions(&mut self, tokens: Vec<InteractionToken>) {
        for token in tokens {
            self.command_intents.remove(&token);
        }
    }
}

fn optional_i32_argument(
    request: &VmHostRequest,
    index: usize,
    default: i32,
) -> Result<i32, RuntimeError> {
    request.argument(index).map_or(Ok(default), |value| {
        i32::try_from(integer_value_or_zero(value)).map_err(|_| RuntimeError::Script {
            kind: erabasic_vm::ScriptFaultKind::Bounds,
            message: format!(
                "host argument {} must fit a signed 32-bit drawing coordinate",
                index + 1
            ),
        })
    })
}

fn optional_opacity(request: &VmHostRequest, index: usize, default: u8) -> u8 {
    let value = request
        .argument(index)
        .map_or(i64::from(default), integer_value_or_zero);
    u8::try_from(value.clamp(0, 255)).expect("clamped opacity fits u8")
}

fn cbg_depth(request: &VmHostRequest, index: usize) -> Result<i64, RuntimeError> {
    optional_cbg_depth(request, index, 0)
}

fn optional_cbg_depth(
    request: &VmHostRequest,
    index: usize,
    default: i64,
) -> Result<i64, RuntimeError> {
    let depth = request
        .argument(index)
        .map_or(default, integer_value_or_zero);
    if depth == 0 || i32::try_from(depth).is_err() {
        return Err(RuntimeError::Script {
            kind: erabasic_vm::ScriptFaultKind::Bounds,
            message: format!(
                "CBG depth argument {} must fit i32 and cannot be zero",
                index + 1
            ),
        });
    }
    Ok(depth)
}

fn optional_color_matrix(
    vm: &RuntimeVm,
    request: &VmHostRequest,
    index: usize,
) -> Result<Option<[i64; 25]>, RuntimeError> {
    request
        .argument(index)
        .map(|value| {
            read_color_matrix(vm, request.fiber, value).and_then(|matrix| {
                matrix.try_into().map_err(|_| {
                    RuntimeError::Internal("graphics color matrix did not contain 25 values".into())
                })
            })
        })
        .transpose()
}
