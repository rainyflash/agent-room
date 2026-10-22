//! 建房间、管房间（`specs/agent-rooms/design.md` 第 3 步）：替这台电脑的账号建私人房间，管 Agent
//! 建的房间怎么进。人建的房间由人在应用里管，控制面把关。

use crate::{
    access::{connect, enter_room, open_store},
    cli::{AccessArgs, AgentArgs, Command, InviteArgs},
    output::{CliFailure as Failure, CliResult as Result, success},
    profile::{EnteredRoom, Invitation, Profile, ProfileStore, RoomTarget, default_display_name},
    required, scoped,
};
use agent_room_agent_client::{
    BridgeToolClient,
    rooms::{BRIDGE_OUTDATED, access_value, bridge_outdated},
};
use agent_room_bridge_ipc::{
    IpcAnswerKnockRequest, IpcCreateRoomRequest, IpcErrorCategory, IpcInvitePersonRequest,
    IpcMethod, IpcRemoveAgentRequest, IpcResponse, IpcRoomAccess, IpcRoomAccessAction,
    IpcRoomAccessRequest, IpcRoomCode, IpcRoomSummary, IpcSelfSummary,
};
use serde_json::{Value, json};
use std::path::Path;

#[cfg(test)]
mod tests;

const PEOPLE: &str = "To bring a person in, ask them to copy their account ID from \"Switch room\" in Agent Room and send it to you, then run invite <account ID>. The room's owner (the account signed in on this computer) can also invite people in the app.";

/// `create-room` 要的：房间名和 Agent 在房间里的名字。
pub(crate) struct Creation {
    pub(crate) room: String,
    pub(crate) name: Option<String>,
}

/// 建房间并进去。同一任务用同一个房间名再建就回到那间，建房的人物也不换：
/// 控制面按房间 ID 认出是同一次建房，建房的 Agent 对不上会拒绝。任务里已经有 Agent（给的名字
/// 是它的，或者没给名字）就由它来建，建好以后它进新房间，会话照旧开在原来的房间。
pub(crate) async fn create(
    backend: &dyn BridgeToolClient,
    root: &Path,
    service: &str,
    selected: Option<String>,
    task_id: Option<String>,
    creation: Creation,
) -> Result<Value> {
    let room_name = creation.room.trim();
    check_creation(room_name, creation.name.as_deref())?;
    let bound = |find: &dyn Fn(&str) -> Option<String>| task_id.as_deref().and_then(find);
    let key = selected
        .clone()
        .or_else(|| bound(&|task| ProfileStore::find_created(root, service, task, room_name)))
        .or_else(|| {
            bound(&|task| ProfileStore::latest_bound(root, service, task, creation.name.as_deref()))
        })
        .unwrap_or_else(|| uuid::Uuid::now_v7().to_string());
    let store = open_store(root, &key).await?;
    let mut profile = match store.load()? {
        Some(profile) => profile,
        None if selected.is_some() => return Err(Failure::validation("cli.profile.not_found")),
        None => creating(
            key.clone(),
            room_name,
            creation.name,
            service,
            task_id.clone(),
        )?,
    };
    profile.validate_binding(service, task_id.as_deref())?;
    // 已有的 Agent 来建：房间 ID 现在就定下，和房间名一起记进档案。
    if profile.created_room.as_deref() != Some(room_name) && profile.created(room_name).is_none() {
        profile.remember(EnteredRoom {
            catalog_id: uuid::Uuid::now_v7().to_string(),
            room_id: None,
            name: Some(room_name.to_owned()),
            created: true,
        });
    }
    // 先记下再去建：结果没回来时重试，回到同一间、同一个人物。
    store.save(&profile)?;
    let entered = enter_created(backend, &store, &mut profile, room_name)
        .await
        .map_err(|mut error| {
            // 没有宿主任务时找不回这个人物，得带着 --profile 重试。
            error.details.insert("profileId".to_owned(), key.clone());
            error
        })?;
    Ok(entered.value(&key, &profile))
}

/// 房间名和显示名先按 IPC 的规矩在本机核对，不合规的不存成人物。
fn check_creation(room: &str, name: Option<&str>) -> Result<()> {
    let probe = IpcCreateRoomRequest {
        catalog_id: uuid::Uuid::now_v7().to_string(),
        name: room.to_owned(),
        session_key: uuid::Uuid::now_v7().to_string(),
        display_name: name.map_or_else(default_display_name, str::to_owned),
    };
    IpcMethod::CreateRoom(probe)
        .validate()
        .map_err(|error| Failure::validation(error.code()))
}

/// 新建房的人物：房间 ID 现在就定下，和房间名一起存进档案。
fn creating(
    key: String,
    room: &str,
    name: Option<String>,
    service: &str,
    task_id: Option<String>,
) -> Result<Profile> {
    let target = RoomTarget {
        catalog_id: Some(uuid::Uuid::now_v7().to_string()),
        room_id: None,
        name: Some(room.to_owned()),
    };
    let invitation = Invitation::for_room(key, name.unwrap_or_else(default_display_name), &target);
    invitation.validate()?;
    let mut profile = Profile::new(invitation, service, task_id);
    profile.created_room = Some(room.to_owned());
    Ok(profile)
}

/// 建好、进去了的房间。
struct Entered {
    identity: IpcSelfSummary,
    room: IpcRoomSummary,
    access: Result<IpcRoomAccess>,
}

impl Entered {
    fn value(self, key: &str, profile: &Profile) -> Value {
        let name = &self.room.name;
        let mut value = json!({
            "profileId": key,
            "identity": self.identity,
            "displayName": profile.invitation.display_name,
            "room": self.room,
            "localAgents": format!("Other agents on this computer (same account) get in with join --room \"{name}\"; agents using MCP call agent_room_join with the room name."),
            "people": PEOPLE,
        });
        let next = match self.access {
            Ok(access) => {
                value["access"] = access_value(&access, Some(name));
                format!(
                    "The room is ready and you are in it. Give the person access.share in their language (zh or en) to pass on to other agents, then keep listening until the person says stop: --profile {key} listen in the background, or --profile {key} read in a loop."
                )
            }
            Err(error) => {
                value["accessError"] = json!(error);
                format!(
                    "The room is ready and you are in it, but its code could not be fetched this time: run --profile {key} access later."
                )
            }
        };
        value["next"] = next.into();
        value
    }
}

/// 请 Bridge 建房（房间 ID 一样就是同一次），照常开会话进去，就绪了顺手看一次口令。
/// 已有的 Agent 建的房间，开好会话以后再进。
async fn enter_created(
    backend: &dyn BridgeToolClient,
    store: &ProfileStore,
    profile: &mut Profile,
    room: &str,
) -> Result<Entered> {
    let home = profile.created_room.as_deref() == Some(room);
    let catalog_id = if home {
        profile.invitation.catalog_id.clone()
    } else {
        profile
            .created(room)
            .map(|created| created.catalog_id.clone())
    }
    .ok_or_else(|| Failure::local("cli.profile.corrupt"))?;
    let request = IpcCreateRoomRequest {
        catalog_id: catalog_id.clone(),
        name: room.to_owned(),
        session_key: profile.invitation.session_key.clone(),
        display_name: profile.invitation.display_name.clone(),
    };
    let IpcResponse::CreatedRoom { room } =
        room_call(backend, IpcMethod::CreateRoom(request)).await?
    else {
        return Err(Failure::local("cli.response_invalid"));
    };
    // 记下 Matrix 房间：连上以后核对进的是这一间。
    if home && profile.invitation.room_id.is_none() && room.matrix_room_id.is_some() {
        profile.invitation.room_id.clone_from(&room.matrix_room_id);
        store.save(profile)?;
    }
    let identity = connect(backend, store, profile).await?;
    let session = required(profile.session_id.clone(), "cli.response_invalid")?;
    let managed = if home {
        profile.current_room = None;
        store.save(profile)?;
        None
    } else {
        let target = RoomTarget {
            catalog_id: Some(catalog_id.clone()),
            room_id: room.matrix_room_id.clone(),
            name: Some(room.name.clone()),
        };
        enter_room(backend, store, profile, target, true).await?;
        Some(catalog_id)
    };
    let access = room_access(backend, session, IpcRoomAccessAction::Show, managed).await;
    Ok(Entered {
        identity,
        room,
        access,
    })
}

/// 管 Agent 建的房间的命令，作用在 `--room` 说的房间上，没说就是最近进的那间。
pub(crate) async fn run(backend: &dyn BridgeToolClient, command: Command) -> Result<()> {
    match command {
        Command::Access(args) => success(access(backend, args).await?),
        Command::Admit(args) => success(answer(backend, args, true).await?),
        Command::Deny(args) => success(answer(backend, args, false).await?),
        Command::Invite(args) => success(invite(backend, args).await?),
        Command::RemoveAgent(args) => success(remove(backend, args).await?),
        // 建房要用 --profile 和宿主任务，在 `crate::run` 里就分出去了。
        _ => Err(Failure::validation("cli.profile.command_unsupported")),
    }
}

/// `access`：口令、房间号、在敲门的 Agent 和要转给别的 Agent 的话。
async fn access(backend: &dyn BridgeToolClient, args: AccessArgs) -> Result<Value> {
    let action = args.action();
    let catalog = managed_room(args.room.as_deref(), args.catalog)?;
    let session = required(args.session, "cli.session_required")?;
    let access = room_access(backend, session, action, catalog).await?;
    let name = room_name(backend, &access.catalog_id).await;
    let mut value = access_value(&access, name.as_deref());
    value["next"] = access_next(&access).into();
    Ok(value)
}

/// `admit`、`deny`：回答敲门的 Agent。
async fn answer(
    backend: &dyn BridgeToolClient,
    args: AgentArgs,
    admit: bool,
) -> Result<IpcResponse> {
    let request = IpcAnswerKnockRequest {
        agent_id: args.agent,
        admit,
        catalog_id: managed_room(args.room.as_deref(), args.catalog)?,
    };
    manage(backend, args.session, IpcMethod::AnswerKnock(request)).await
}

/// `invite`：按账户 ID 邀请一个人，给查看、发言和自动发言。
async fn invite(backend: &dyn BridgeToolClient, args: InviteArgs) -> Result<IpcResponse> {
    let request = IpcInvitePersonRequest {
        account_id: args.account.trim().to_ascii_lowercase(),
        catalog_id: managed_room(args.room.as_deref(), args.catalog)?,
    };
    manage(backend, args.session, IpcMethod::InvitePerson(request)).await
}

/// `remove-agent`：移出凭口令或敲门进来的 Agent。
async fn remove(backend: &dyn BridgeToolClient, args: AgentArgs) -> Result<IpcResponse> {
    let request = IpcRemoveAgentRequest {
        agent_id: args.agent,
        catalog_id: managed_room(args.room.as_deref(), args.catalog)?,
    };
    manage(backend, args.session, IpcMethod::RemoveAgent(request)).await
}

/// 管哪个房间：带着档案时已经按 `--room` 换好了目录 ID；只给会话时 `--room` 要是目录 ID。
fn managed_room(room: Option<&str>, catalog: Option<String>) -> Result<Option<String>> {
    match (catalog, room.map(str::trim)) {
        (Some(catalog), _) => Ok(Some(catalog)),
        (None, None) => Ok(None),
        (None, Some(room)) if uuid::Uuid::parse_str(room).is_ok() => Ok(Some(room.to_owned())),
        (None, Some(_)) => Err(Failure::validation("cli.profile.required")),
    }
}

async fn manage(
    backend: &dyn BridgeToolClient,
    session: Option<String>,
    method: IpcMethod,
) -> Result<IpcResponse> {
    let session = required(session, "cli.session_required")?;
    match room_call(backend, scoped(session, method)).await? {
        response @ IpcResponse::RoomUpdated { .. } => Ok(response),
        _ => Err(Failure::local("cli.response_invalid")),
    }
}

async fn room_access(
    backend: &dyn BridgeToolClient,
    session: String,
    action: IpcRoomAccessAction,
    catalog_id: Option<String>,
) -> Result<IpcRoomAccess> {
    let method = IpcMethod::RoomAccess(IpcRoomAccessRequest { action, catalog_id });
    match room_call(backend, scoped(session, method)).await? {
        IpcResponse::RoomAccess { access } => Ok(access),
        _ => Err(Failure::local("cli.response_invalid")),
    }
}

/// 房间名只用在要转发的话里；列房间找不到时就不写名字。
async fn room_name(backend: &dyn BridgeToolClient, catalog_id: &str) -> Option<String> {
    match backend.invoke(IpcMethod::ListRooms).await {
        Ok(IpcResponse::Rooms { rooms }) => rooms
            .into_iter()
            .find(|room| room.catalog_id == catalog_id)
            .map(|room| room.name),
        _ => None,
    }
}

/// 建房、管房间的调用。旧版 Bridge 不认这个授权范围，握手就断开：照实说要更新桌面端。
async fn room_call(backend: &dyn BridgeToolClient, method: IpcMethod) -> Result<IpcResponse> {
    method
        .validate()
        .map_err(|error| Failure::validation(error.code()))?;
    backend.invoke(method).await.map_err(|failure| {
        if bridge_outdated(&failure) {
            let mut error = Failure::local(BRIDGE_OUTDATED);
            error.category = IpcErrorCategory::IncompatibleVersion;
            error
        } else {
            failure.into()
        }
    })
}

fn access_next(access: &IpcRoomAccess) -> &'static str {
    if !access.knocks.is_empty() {
        return "Agents are knocking (knocks): ask the person whether to let each one in, then run admit <agentId> or deny <agentId>.";
    }
    match access.code {
        IpcRoomCode::Visible { .. } => {
            "Give the person share in their language (zh or en) to pass on to other agents; agents on this computer get in with join --room and the room name."
        }
        IpcRoomCode::Hidden => {
            "The code was created earlier and can no longer be shown, so share only explains knocking. To give agents a new code, ask the person first, then run access --replace-code: the old code stops working, and agents already inside stay."
        }
        IpcRoomCode::Missing => {
            "The code is turned off, so share only explains knocking. Run access without options to create a new code."
        }
    }
}
