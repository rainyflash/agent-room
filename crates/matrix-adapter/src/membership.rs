use std::{fmt, sync::Arc};

use agent_room_application::ports::{
    MatrixFailure, MatrixFailureKind, MatrixGateway, MatrixOperation, MatrixResult, MatrixRoomId,
    PortFuture, RoomMembershipGateway,
};
use agent_room_domain::rooms::MatrixRoomReference;

#[derive(Clone)]
pub struct MatrixRoomMembershipAdapter {
    gateway: Arc<dyn MatrixGateway>,
}

impl MatrixRoomMembershipAdapter {
    pub const fn new(gateway: Arc<dyn MatrixGateway>) -> Self {
        Self { gateway }
    }
}

impl fmt::Debug for MatrixRoomMembershipAdapter {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MatrixRoomMembershipAdapter")
            .finish_non_exhaustive()
    }
}

impl RoomMembershipGateway for MatrixRoomMembershipAdapter {
    fn join<'a>(&'a self, room_id: &'a MatrixRoomReference) -> PortFuture<'a, MatrixResult<()>> {
        Box::pin(async move {
            let room_id = to_matrix_room_id(room_id, MatrixOperation::Join)?;
            self.gateway.join(&room_id).await
        })
    }

    fn leave<'a>(&'a self, room_id: &'a MatrixRoomReference) -> PortFuture<'a, MatrixResult<()>> {
        Box::pin(async move {
            let room_id = to_matrix_room_id(room_id, MatrixOperation::Leave)?;
            self.gateway.leave(&room_id).await
        })
    }
}

fn to_matrix_room_id(
    room_id: &MatrixRoomReference,
    operation: MatrixOperation,
) -> MatrixResult<MatrixRoomId> {
    MatrixRoomId::new(room_id.as_str().to_owned())
        .map_err(|_| MatrixFailure::new(operation, MatrixFailureKind::InvalidResponse))
}
