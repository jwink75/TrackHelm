<script lang="ts">
  let appState = "STANDBY";
  let activeProject = "None";
</script>

<main class="container">
  <header class="lcars-header">
    <div class="lcars-bar-left"></div>
    <div class="lcars-title-block">
      <span class="system-id">SYS-9472</span>
      <h1>TRACKHELM // WORKSTATION</h1>
    </div>
    <div class="lcars-bar-right">
      <span class="status-indicator">{appState}</span>
    </div>
  </header>

  <div class="content-grid">
    <aside class="sidebar">
      <button class="nav-btn active">WORKSPACE</button>
      <button class="nav-btn">LIBRARY</button>
      <button class="nav-btn">OSC/MIDI</button>
      <button class="nav-btn">SETTINGS</button>
    </aside>

    <section class="main-display">
      <div class="status-panel">
        <p>ACTIVE PROJECT: <span class="highlight">{activeProject}</span></p>
        <p>DSP ENGINE: <span class="highlight">OFFLINE</span></p>
      </div>
      <div class="welcome-msg">
        <p>System initialized. Ready for Milestone 1 playback engine implementation.</p>
      </div>
    </section>
  </div>
</main>

<style>
  :global(body) {
    background-color: #000000;
    color: #ff9900; /* LCARS gold */
    font-family: "Courier New", Courier, monospace;
    margin: 0;
    padding: 0;
  }

  .container {
    display: flex;
    flex-direction: column;
    height: 100vh;
    padding: 10px;
    box-sizing: border-box;
  }

  .lcars-header {
    display: flex;
    align-items: flex-end;
    margin-bottom: 20px;
  }

  .lcars-bar-left {
    background-color: #99ccff; /* LCARS blue */
    width: 60px;
    height: 40px;
    border-radius: 20px 0 0 20px;
  }

  .lcars-title-block {
    background-color: #ff9900; /* LCARS gold */
    color: #000000;
    padding: 2px 20px;
    flex-grow: 1;
    display: flex;
    justify-content: space-between;
    align-items: center;
    font-weight: bold;
    height: 36px;
  }

  .system-id {
    font-size: 0.8rem;
    color: #444444;
  }

  h1 {
    margin: 0;
    font-size: 1.2rem;
  }

  .lcars-bar-right {
    background-color: #ffcc00; /* LCARS yellow */
    width: 120px;
    height: 40px;
    border-radius: 0 20px 20px 0;
    display: flex;
    align-items: center;
    justify-content: center;
  }

  .status-indicator {
    color: #000000;
    font-weight: bold;
    font-size: 0.9rem;
  }

  .content-grid {
    display: flex;
    flex-grow: 1;
    gap: 15px;
  }

  .sidebar {
    width: 150px;
    display: flex;
    flex-direction: column;
    gap: 10px;
  }

  .nav-btn {
    background-color: #cc6699; /* LCARS violet-pink */
    color: #000000;
    border: none;
    border-radius: 10px;
    padding: 10px;
    font-weight: bold;
    text-align: right;
    cursor: pointer;
    transition: background-color 0.2s;
  }

  .nav-btn:hover {
    background-color: #ff99cc;
  }

  .nav-btn.active {
    background-color: #99ccff;
  }

  .main-display {
    flex-grow: 1;
    border: 2px solid #ffcc00;
    border-radius: 15px;
    padding: 20px;
    display: flex;
    flex-direction: column;
    gap: 20px;
  }

  .status-panel {
    border-bottom: 1px solid #444444;
    padding-bottom: 10px;
  }

  .highlight {
    color: #99ccff;
  }

  .welcome-msg {
    font-size: 1.1rem;
    line-height: 1.6;
    color: #dddddd;
  }
</style>
