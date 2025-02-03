use bevy::{
    ecs::system::{FilteredResourcesMutParamBuilder, FilteredResourcesParamBuilder, ParamBuilder},
    prelude::*,
};

use super::{server_tick::ServerTick, ServerSet};
use crate::core::{
    common_conditions::*,
    connected_clients::ConnectedClients,
    event::{
        ctx::{ServerReceiveCtx, ServerSendCtx},
        event_registry::EventRegistry,
        server_event::BufferedServerEvents,
    },
    replication::replicated_clients::ReplicatedClients,
    replicon_server::RepliconServer,
};

/// Sending events from the server to clients.
///
/// Requires [`ServerPlugin`](super::ServerPlugin).
/// Can be disabled for apps that act only as clients.
pub struct ServerEventPlugin;

impl Plugin for ServerEventPlugin {
    fn build(&self, _app: &mut App) {}

    fn finish(&self, app: &mut App) {
        // Construct systems dynamically after all plugins initialization
        // because we need to access resources by registered IDs.
        let event_registry = app
            .world_mut()
            .remove_resource::<EventRegistry>()
            .expect("event registry should be initialized on app build");

        let send_or_buffer = (
            FilteredResourcesParamBuilder::new(|builder| {
                for event in event_registry.iter_server_events() {
                    builder.add_read_by_id(event.server_events_id());
                }
            }),
            ParamBuilder,
            ParamBuilder,
            ParamBuilder,
            ParamBuilder,
            ParamBuilder,
        )
            .build_state(app.world_mut())
            .build_system(send_or_buffer);

        let receive = (
            FilteredResourcesMutParamBuilder::new(|builder| {
                for event in event_registry.iter_client_events() {
                    builder.add_write_by_id(event.client_events_id());
                }
            }),
            ParamBuilder,
            ParamBuilder,
            ParamBuilder,
        )
            .build_state(app.world_mut())
            .build_system(receive);

        let trigger = (
            FilteredResourcesMutParamBuilder::new(|builder| {
                for trigger in event_registry.iter_client_triggers() {
                    builder.add_write_by_id(trigger.event().client_events_id());
                }
            }),
            ParamBuilder,
            ParamBuilder,
        )
            .build_state(app.world_mut())
            .build_system(trigger);

        let resend_locally = (
            FilteredResourcesMutParamBuilder::new(|builder| {
                for event in event_registry.iter_server_events() {
                    builder.add_write_by_id(event.server_events_id());
                }
            }),
            FilteredResourcesMutParamBuilder::new(|builder| {
                for event in event_registry.iter_server_events() {
                    builder.add_write_by_id(event.events_id());
                }
            }),
            ParamBuilder,
        )
            .build_state(app.world_mut())
            .build_system(resend_locally);

        app.insert_resource(event_registry)
            .add_systems(
                PreUpdate,
                (
                    receive.run_if(server_running),
                    trigger.run_if(server_or_singleplayer),
                )
                    .chain()
                    .in_set(ServerSet::Receive),
            )
            .add_systems(
                PostUpdate,
                (
                    send_or_buffer.run_if(server_running),
                    send_buffered
                        .run_if(server_running)
                        .run_if(resource_changed::<ServerTick>),
                    resend_locally.run_if(server_or_singleplayer),
                )
                    .chain()
                    .after(super::send_replication)
                    .in_set(ServerSet::Send),
            );
    }
}

fn send_or_buffer(
    server_events: FilteredResources,
    mut server: ResMut<RepliconServer>,
    mut buffered_events: ResMut<BufferedServerEvents>,
    registry: Res<AppTypeRegistry>,
    connected_clients: Res<ConnectedClients>,
    event_registry: Res<EventRegistry>,
) {
    buffered_events.start_tick();
    let mut ctx = ServerSendCtx {
        registry: &registry.read(),
    };

    for event in event_registry.iter_server_events() {
        let server_events = server_events
            .get_by_id(event.server_events_id())
            .expect("server events resource should be accessible");

        // SAFETY: passed pointer was obtained using this event data.
        unsafe {
            event.send_or_buffer(
                &mut ctx,
                &server_events,
                &mut server,
                &connected_clients,
                &mut buffered_events,
            );
        }
    }
}

fn send_buffered(
    mut server: ResMut<RepliconServer>,
    mut buffered_events: ResMut<BufferedServerEvents>,
    replicated_clients: Res<ReplicatedClients>,
) {
    buffered_events
        .send_all(&mut server, &replicated_clients)
        .expect("buffered server events should send");
}

fn receive(
    mut client_events: FilteredResourcesMut,
    mut server: ResMut<RepliconServer>,
    registry: Res<AppTypeRegistry>,
    event_registry: Res<EventRegistry>,
) {
    let mut ctx = ServerReceiveCtx {
        registry: &registry.read(),
    };

    for event in event_registry.iter_client_events() {
        let client_events = client_events
            .get_mut_by_id(event.client_events_id())
            .expect("client events resource should be accessible");

        // SAFETY: passed pointer was obtained using this event data.
        unsafe { event.receive(&mut ctx, client_events.into_inner(), &mut server) };
    }
}

fn trigger(
    mut client_events: FilteredResourcesMut,
    mut commands: Commands,
    event_registry: Res<EventRegistry>,
) {
    for trigger in event_registry.iter_client_triggers() {
        let client_events = client_events
            .get_mut_by_id(trigger.event().client_events_id())
            .expect("client events resource should be accessible");
        trigger.trigger(&mut commands, client_events.into_inner());
    }
}

fn resend_locally(
    mut server_events: FilteredResourcesMut,
    mut events: FilteredResourcesMut,
    event_registry: Res<EventRegistry>,
) {
    for event in event_registry.iter_server_events() {
        let server_events = server_events
            .get_mut_by_id(event.server_events_id())
            .expect("server events resource should be accessible");
        let events = events
            .get_mut_by_id(event.events_id())
            .expect("events resource should be accessible");

        // SAFETY: passed pointers were obtained using this event data.
        unsafe { event.resend_locally(server_events.into_inner(), events.into_inner()) };
    }
}
