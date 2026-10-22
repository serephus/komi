// Live updates over server-sent events.
//
// Every page subscribes to one stream. An event says "<game id> <kind>"; a game
// page reloads when the event is about its game, and the lobby (which has no
// game id) reloads on any event. Reloading the page is deliberate: the server
// renders it from the database, so a fresh render cannot disagree with the
// record, and no client-side patching has to be kept correct.
//
// EventSource reconnects by itself. The stream starts with a "hello": a second
// one means the connection dropped and came back, so the page may have missed
// events in between and reloads.
//
// Without JavaScript, or in a browser without EventSource, everything still
// works — a move is an ordinary form post. The game page additionally falls back
// to occasionally asking /g/{id}/version.
document.addEventListener("DOMContentLoaded", () => {
  const marker = document.querySelector("[data-stream]");
  if (!marker) return;

  const streamUrl = marker.getAttribute("data-stream");
  const gameId = marker.getAttribute("data-game") || "";
  const fallbackUrl = marker.getAttribute("data-fallback");
  const seq = marker.getAttribute("data-seq");
  const refreshMs = Math.max(5, Number(marker.getAttribute("data-refresh")) || 30) * 1000;

  const reload = () => window.location.reload();

  const pollOnce = async () => {
    if (!fallbackUrl) return;
    try {
      const response = await fetch(fallbackUrl, { cache: "no-store", credentials: "same-origin" });
      if (!response.ok) return;
      const state = await response.json();
      if (String(state.seq) !== seq || state.state === "finished") reload();
    } catch {
      // Offline for a moment; the next tick will try again.
    }
  };

  if (!("EventSource" in window)) {
    if (fallbackUrl) {
      window.setInterval(pollOnce, refreshMs);
      document.addEventListener("visibilitychange", () => {
        if (!document.hidden) pollOnce();
      });
    }
    return;
  }

  let connected = false;
  const source = new EventSource(streamUrl);

  source.addEventListener("hello", () => {
    if (connected) reload();
    connected = true;
  });

  source.addEventListener("game", (event) => {
    const [id] = String(event.data).split(" ");
    if (gameId === "" || id === gameId) reload();
  });

  // The server dropped events for this connection: reload rather than sit on a
  // stale board.
  source.addEventListener("resync", () => reload());
});
