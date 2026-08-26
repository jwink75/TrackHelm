// TrackHelm Stream Deck Controller Plugin
let elgatoWs = null;
let trackhelmWs = null;
let pluginUUID = null;

// Track active visible button contexts
const activeButtons = new Map(); // context -> { action, state }

// Latest TrackHelm state
let latestState = {
  isPlaying: false,
  currentTime: 0,
  duration: 0,
  formattedTime: "00:00.00",
  formattedRemaining: "00:00.00",
  trackName: "",
  currentMarker: "",
  pitchSemitones: 0,
  volumeDb: 0,
  speed: 1.0,
  isLooping: false,
};

// 1. Entry point called by Stream Deck Application
function connectElgatoStreamDeckSocket(inPort, inPluginUUID, inRegisterEvent, inInfo) {
  pluginUUID = inPluginUUID;
  elgatoWs = new WebSocket("ws://127.0.0.1:" + inPort);

  elgatoWs.onopen = function () {
    const registerData = {
      event: inRegisterEvent,
      uuid: inPluginUUID,
    };
    elgatoWs.send(JSON.stringify(registerData));
    initTrackHelmConnection();
  };

  elgatoWs.onmessage = function (evt) {
    const jsonObj = JSON.parse(evt.data);
    const event = jsonObj.event;
    const action = jsonObj.action;
    const context = jsonObj.context;

    if (event === "willAppear") {
      activeButtons.set(context, { action: action, context: context });
      updateButtonDisplay(context, action);
    } else if (event === "willDisappear") {
      activeButtons.delete(context);
    } else if (event === "keyDown") {
      handleStreamDeckKeyDown(action, context);
    }
  };
}

// 2. Connect to TrackHelm Backend Engine via WebSocket (ws://127.0.0.1:4545)
function initTrackHelmConnection() {
  if (trackhelmWs) {
    try { trackhelmWs.close(); } catch (e) {}
  }

  trackhelmWs = new WebSocket("ws://127.0.0.1:4545");

  trackhelmWs.onopen = function () {
    console.log("[StreamDeck] Connected to TrackHelm on port 4545");
  };

  trackhelmWs.onmessage = function (evt) {
    try {
      const data = JSON.parse(evt.data);
      if (data.type === "state") {
        latestState = { ...latestState, ...data };
        updateAllVisibleButtons();
      }
    } catch (e) {
      console.error("[StreamDeck] Error parsing TrackHelm message:", e);
    }
  };

  trackhelmWs.onclose = function () {
    console.log("[StreamDeck] Disconnected from TrackHelm. Retrying in 2s...");
    setTimeout(initTrackHelmConnection, 2000);
  };

  trackhelmWs.onerror = function () {
    trackhelmWs.close();
  };
}

// 3. Handle Key Presses on the Stream Deck
function handleStreamDeckKeyDown(action, context) {
  if (!trackhelmWs || trackhelmWs.readyState !== WebSocket.OPEN) {
    showElgatoAlert(context);
    return;
  }

  let command = null;
  switch (action) {
    case "com.trackhelm.controller.playpause":
      command = "play_pause";
      break;
    case "com.trackhelm.controller.rewind":
      command = "rewind";
      break;
    case "com.trackhelm.controller.nexttrack":
      command = "next_track";
      break;
    case "com.trackhelm.controller.prevtrack":
      command = "prev_track";
      break;
    case "com.trackhelm.controller.nextmarker":
      command = "next_marker";
      break;
    case "com.trackhelm.controller.prevmarker":
      command = "prev_marker";
      break;
    case "com.trackhelm.controller.addmarker":
      command = "add_marker";
      break;
    case "com.trackhelm.controller.pitchup":
      command = "pitch_up";
      break;
    case "com.trackhelm.controller.pitchdown":
      command = "pitch_down";
      break;
    case "com.trackhelm.controller.volup":
      command = "volume_up";
      break;
    case "com.trackhelm.controller.voldown":
      command = "volume_down";
      break;
    case "com.trackhelm.controller.speedup":
      command = "speed_up";
      break;
    case "com.trackhelm.controller.speeddown":
      command = "speed_down";
      break;
    case "com.trackhelm.controller.loop":
      command = "toggle_loop";
      break;
    case "com.trackhelm.controller.cut":
      command = "toggle_cut";
      break;
    default:
      break;
  }

  if (command) {
    trackhelmWs.send(command);
  }
}

// 4. Update Dynamic Titles and States on Stream Deck LCD Buttons
function updateAllVisibleButtons() {
  activeButtons.forEach((btn, context) => {
    updateButtonDisplay(context, btn.action);
  });
}

function updateButtonDisplay(context, action) {
  if (!elgatoWs || elgatoWs.readyState !== WebSocket.OPEN) return;

  switch (action) {
    case "com.trackhelm.controller.playpause":
      const stateIdx = latestState.isPlaying ? 1 : 0;
      setElgatoState(context, stateIdx);
      setElgatoTitle(context, latestState.isPlaying ? `▶\n${latestState.formattedTime}` : `⏸\n${latestState.formattedTime}`);
      break;

    case "com.trackhelm.controller.songinfo":
      const shortName = (latestState.trackName || "No Track").slice(0, 14);
      setElgatoTitle(context, `${shortName}\n${latestState.formattedTime}`);
      break;

    case "com.trackhelm.controller.markerinfo":
      const markerText = latestState.currentMarker ? latestState.currentMarker.slice(0, 12) : "No Marker";
      setElgatoTitle(context, `⚐ ${markerText}\n${latestState.formattedTime}`);
      break;

    case "com.trackhelm.controller.pitchup":
    case "com.trackhelm.controller.pitchdown":
      const semi = latestState.pitchSemitones;
      setElgatoTitle(context, `${semi > 0 ? "+" : ""}${semi} st`);
      break;

    case "com.trackhelm.controller.volup":
    case "com.trackhelm.controller.voldown":
      const vol = latestState.volumeDb;
      setElgatoTitle(context, `${vol > 0 ? "+" : ""}${vol.toFixed(1)} dB`);
      break;

    case "com.trackhelm.controller.speedup":
    case "com.trackhelm.controller.speeddown":
      setElgatoTitle(context, `${latestState.speed.toFixed(2)}x`);
      break;

    default:
      break;
  }
}

// Stream Deck Helper Functions
function setElgatoTitle(context, title) {
  if (!elgatoWs || elgatoWs.readyState !== WebSocket.OPEN) return;
  elgatoWs.send(JSON.stringify({
    event: "setTitle",
    context: context,
    payload: {
      title: title,
      target: 0
    }
  }));
}

function setElgatoState(context, state) {
  if (!elgatoWs || elgatoWs.readyState !== WebSocket.OPEN) return;
  elgatoWs.send(JSON.stringify({
    event: "setState",
    context: context,
    payload: {
      state: state
    }
  }));
}

function showElgatoAlert(context) {
  if (!elgatoWs || elgatoWs.readyState !== WebSocket.OPEN) return;
  elgatoWs.send(JSON.stringify({
    event: "showAlert",
    context: context
  }));
}
