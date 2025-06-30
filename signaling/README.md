# WebRTC Signaling Server with Rheomesh SFU

A high-performance WebRTC signaling server built with Rust using [Rheomesh](https://github.com/h3poteto/rheomesh) v0.5.0 for SFU (Selective Forwarding Unit) capabilities.

## Features

- **Full SFU Support**: Built on Rheomesh v0.5.0 for efficient media forwarding
- **WebSocket Signaling**: Real-time bidirectional communication
- **JWT Authentication**: Secure user authentication
- **Room-based Architecture**: Multi-room support with participant management
- **Redis Clustering**: Optional horizontal scaling with Redis coordination
- **Video & Audio Streaming**: Support for multiple media streams
- **ICE Candidate Handling**: Full WebRTC connection establishment
- **Publish/Subscribe Model**: Users can publish streams and subscribe to others

## Architecture

### SFU (Selective Forwarding Unit)

Unlike peer-to-peer WebRTC where clients connect directly to each other, this SFU server:

- Receives media streams from publishers
- Forwards streams to subscribers without transcoding
- Reduces bandwidth usage for participants
- Enables better scalability for group calls

### Message Flow

1. **Publishing**: Client sends video/audio to SFU
2. **Forwarding**: SFU distributes media to subscribed clients
3. **No Direct P2P**: All media flows through the SFU server

## Quick Start

### 1. Environment Setup

```bash
# Copy environment template
cp ../env.example .env

# Set required variables
export JWT_SECRET="your-super-secret-jwt-key"
export REDIS_URL="redis://localhost:6379"
export CLUSTER_MODE="false"  # Set to true for multi-server setup
```

### 2. Run with Docker

```bash
# Start the full stack (includes Redis, PostgreSQL, backend)
docker-compose up -d

# Or run signaling server only
docker-compose up signaling
```

### 3. Run in Development

```bash
# Install Rust dependencies
cargo build

# Run the server
RUST_LOG=debug cargo run -- --host 0.0.0.0
```

## WebSocket API

### Authentication

First message must be authentication:

```json
{
  "type": "auth",
  "token": "your-jwt-token"
}
```

### Publishing Video/Audio

#### 1. Send Publish Offer

```json
{
  "type": "publish-offer",
  "roomName": "my-room",
  "offer": "v=0\r\no=- 123... (SDP offer)"
}
```

#### 2. Receive Publish Answer

```json
{
  "type": "publish-answer",
  "roomName": "my-room",
  "answer": "v=0\r\na=... (SDP answer)"
}
```

#### 3. Exchange ICE Candidates

```json
{
  "type": "publish-ice-candidate",
  "roomName": "my-room",
  "candidate": "candidate:1 1 UDP..."
}
```

### Subscribing to Other Users

#### 1. Request Subscription

```json
{
  "type": "subscribe-request",
  "roomName": "my-room",
  "publisherId": 123
}
```

#### 2. Receive Subscribe Offer

```json
{
  "type": "subscribe-offer",
  "roomName": "my-room",
  "publisherId": 123,
  "offer": "v=0\r\no=- 456... (SDP offer)"
}
```

#### 3. Send Subscribe Answer

```json
{
  "type": "subscribe-answer",
  "roomName": "my-room",
  "publisherId": 123,
  "answer": "v=0\r\na=... (SDP answer)"
}
```

### Room Events

#### New Publisher Available

```json
{
  "type": "new-publisher",
  "roomName": "my-room",
  "publisherId": 123,
  "username": "john_doe"
}
```

#### Publisher Left

```json
{
  "type": "publisher-left",
  "roomName": "my-room",
  "publisherId": 123
}
```

## Client Integration Example

### JavaScript/TypeScript Client

```javascript
const ws = new WebSocket("ws://localhost:3002");

// 1. Authenticate
ws.send(
  JSON.stringify({
    type: "auth",
    token: "your-jwt-token",
  })
);

// 2. Get user media
const stream = await navigator.mediaDevices.getUserMedia({
  video: true,
  audio: true,
});

// 3. Create peer connection
const pc = new RTCPeerConnection({
  iceServers: [{ urls: "stun:stun.l.google.com:19302" }],
});

// 4. Add local stream
stream.getTracks().forEach((track) => {
  pc.addTrack(track, stream);
});

// 5. Create and send offer
const offer = await pc.createOffer();
await pc.setLocalDescription(offer);

ws.send(
  JSON.stringify({
    type: "publish-offer",
    roomName: "my-room",
    offer: offer.sdp,
  })
);

// 6. Handle answer
ws.onmessage = async (event) => {
  const message = JSON.parse(event.data);

  if (message.type === "publish-answer") {
    await pc.setRemoteDescription({
      type: "answer",
      sdp: message.answer,
    });
  }

  if (message.type === "new-publisher") {
    // Subscribe to new publisher
    ws.send(
      JSON.stringify({
        type: "subscribe-request",
        roomName: message.roomName,
        publisherId: message.publisherId,
      })
    );
  }
};

// 7. Handle ICE candidates
pc.onicecandidate = (event) => {
  if (event.candidate) {
    ws.send(
      JSON.stringify({
        type: "publish-ice-candidate",
        roomName: "my-room",
        candidate: event.candidate.candidate,
      })
    );
  }
};
```

### Frontend Integration

The server is designed to work with the existing Vue.js frontend. Update your WebSocket service:

```typescript
// services/websocket.ts
export enum MessageType {
  // ... existing types
  PublishOffer = "publish-offer",
  PublishAnswer = "publish-answer",
  PublishIceCandidate = "publish-ice-candidate",
  SubscribeRequest = "subscribe-request",
  SubscribeOffer = "subscribe-offer",
  SubscribeAnswer = "subscribe-answer",
  SubscribeIceCandidate = "subscribe-ice-candidate",
  NewPublisher = "new-publisher",
  PublisherLeft = "publisher-left",
}
```

## Configuration

### Environment Variables

- `JWT_SECRET`: Secret key for JWT token validation (required)
- `PORT`: Server port (default: 9000)
- `RUST_LOG`: Log level (debug, info, warn, error)
- `CLUSTER_MODE`: Enable Redis clustering (default: false)
- `REDIS_URL`: Redis connection string for clustering
- `NODE_ID`: Unique server identifier for clustering

### Clustering

For horizontal scaling across multiple servers:

```bash
# Server 1
export CLUSTER_MODE=true
export NODE_ID=signaling-1
export REDIS_URL=redis://redis-server:6379

# Server 2
export CLUSTER_MODE=true
export NODE_ID=signaling-2
export REDIS_URL=redis://redis-server:6379
```

## Performance Characteristics

### SFU Benefits

- **Bandwidth Efficiency**: Each client uploads once, downloads what they need
- **CPU Efficiency**: No transcoding, just forwarding
- **Scalability**: Better than mesh topology for 3+ participants
- **Quality Control**: Per-subscriber bitrate adaptation

### Resource Usage

- **Memory**: ~10MB base + ~1MB per active connection
- **CPU**: Minimal (mostly network I/O)
- **Network**: Proportional to (publishers × subscribers)

## Development

### Project Structure

```
signaling/
├── src/
│   ├── auth.rs              # JWT authentication
│   ├── cluster.rs           # Redis clustering
│   ├── messages.rs          # WebSocket message definitions
│   ├── rheomesh_sfu.rs      # SFU implementation with Rheomesh
│   ├── room.rs              # Room management
│   ├── server.rs            # WebSocket server
│   └── main.rs              # Application entry point
├── Dockerfile               # Container build
└── README.md               # This file
```

### Adding Features

1. **New Message Types**: Add to `messages.rs` and handle in `server.rs`
2. **SFU Features**: Extend `rheomesh_sfu.rs` with Rheomesh capabilities
3. **Room Logic**: Modify room management in `room.rs`

### Testing

```bash
# Run unit tests
cargo test

# Run with debug logging
RUST_LOG=debug cargo run

# Test WebSocket connection
websocat ws://localhost:9000
```

## Rheomesh Features Used

### Core Capabilities

- **Router**: Manages media routing within rooms
- **PublishTransport**: Handles incoming media streams
- **SubscribeTransport**: Manages outgoing media streams
- **Worker**: Coordinates multiple routers

### Advanced Features Available

- **Simulcast**: Multiple quality streams from single publisher
- **SVC**: Scalable Video Coding support
- **DataChannels**: Non-media data transmission
- **Recording**: Stream recording capabilities (future)
- **Relay**: Cross-server media forwarding (future)

## Comparison with LiveKit

| Feature           | This Implementation | LiveKit        |
| ----------------- | ------------------- | -------------- |
| **Language**      | Rust (Performance)  | Go             |
| **SFU Library**   | Rheomesh            | Custom         |
| **Signaling**     | Custom WebSocket    | gRPC/WebSocket |
| **Complexity**    | Simplified          | Full-featured  |
| **Customization** | High                | Medium         |
| **Performance**   | Very High           | High           |
| **Community**     | Growing             | Established    |

## Troubleshooting

### Common Issues

#### WebSocket Connection Fails

```bash
# Check server is running
curl -f http://localhost:9000/health || echo "Server not responding"

# Check authentication
echo '{"type":"auth","token":"invalid"}' | websocat ws://localhost:9000
```

#### SFU Not Available

- Ensure Rheomesh dependency is properly installed
- Check `RUST_LOG=debug` output for SFU initialization errors
- Verify WebRTC ICE servers are accessible

#### Media Not Flowing

- Verify ICE candidates are being exchanged
- Check firewall rules for UDP ports
- Ensure STUN servers are reachable
- Monitor SFU logs for media transport errors

### Debug Mode

```bash
RUST_LOG=debug cargo run
```

### Performance Monitoring

The server exposes metrics for monitoring:

- Active connections
- Room statistics
- SFU transport status
- Redis cluster health (if enabled)

## Contributing

1. Fork the repository
2. Create a feature branch
3. Add tests for new functionality
4. Ensure `cargo test` passes
5. Submit a pull request

## License

This project follows the same license as the main application.
