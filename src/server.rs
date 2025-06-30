use rheomesh::sdp::RTCSessionDescription;
use rheomesh::transport::Transport;

// ... existing code ...

// ... existing code ...

// In handle_client_message:
// - For get_answer/set_answer, convert sdp string to RTCSessionDescription using RTCSessionDescription::offer_from_sdp or answer_from_sdp.
// - For answer, use answer.sdp() to get the string to send back.
// - For ICE, use on_ice_candidate instead of add_ice_candidate.
// Remove unused imports: LocalRoomManager, Room, Rooms, HashMap, RwLock.

// ... existing code ...
