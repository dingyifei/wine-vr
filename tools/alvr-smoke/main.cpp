// Stage-0 smoke test for embedding alvr_server_core (ALVR v20.14.1) on macOS.
// Success gate: ClientConnected + TrackingUpdated events from a stock ALVR
// Quest client, with no oxrsys involvement.
//
// Usage: alvr-smoke <config_dir> <log_dir>
// Writes a minimal session.json enabling client discovery with
// auto_trust_clients, so any v20 client on the LAN can connect untouched.

#include "alvr_server_core.h"

#include <atomic>
#include <csignal>
#include <cstdio>
#include <cstring>
#include <filesystem>
#include <fstream>
#include <string>
#include <vector>

namespace fs = std::filesystem;

static std::atomic<bool> g_running{true};

static void OnSignal(int) { g_running = false; }

static const char* kMinimalSessionJson = R"({
  "session_settings": {
    "connection": {
      "client_discovery": { "enabled": true, "content": { "auto_trust_clients": true } }
    }
  }
})";

int main(int argc, char** argv) {
    setvbuf(stdout, nullptr, _IOLBF, 0); // live output when redirected to a file

    if (argc < 3) {
        std::fprintf(stderr, "usage: %s <config_dir> <log_dir>\n", argv[0]);
        return 2;
    }
    const std::string configDir = argv[1];
    const std::string logDir = argv[2];
    fs::create_directories(configDir);
    fs::create_directories(logDir);

    const fs::path sessionPath = fs::path(configDir) / "session.json";
    if (!fs::exists(sessionPath)) {
        std::ofstream out(sessionPath);
        out << kMinimalSessionJson;
        std::printf("[smoke] wrote minimal session.json (discovery + auto-trust)\n");
    } else {
        std::printf("[smoke] using existing session.json\n");
    }

    std::signal(SIGINT, OnSignal);
    std::signal(SIGTERM, OnSignal);

    std::printf("[smoke] alvr_initialize_environment(%s, %s)\n", configDir.c_str(), logDir.c_str());
    alvr_initialize_environment(configDir.c_str(), logDir.c_str());
    alvr_initialize_logging(nullptr, nullptr);

    const AlvrTargetConfig target = alvr_initialize();
    std::printf("[smoke] alvr_initialize: game_render=%ux%u stream=%ux%u\n",
                target.game_render_width, target.game_render_height, target.stream_width,
                target.stream_height);

    uint64_t settingsLen = alvr_get_settings_json(nullptr);
    std::vector<char> settings(settingsLen);
    alvr_get_settings_json(settings.data());
    std::printf("[smoke] settings json: %llu bytes\n", (unsigned long long)settingsLen);

    alvr_start_connection();
    std::printf("[smoke] connection started; waiting for a stock ALVR client on the LAN...\n");
    std::printf("[smoke] (headset: ALVR client open, same WiFi; Ctrl-C to exit)\n");

    bool sawClientConnected = false;
    bool sawTracking = false;
    uint64_t trackingCount = 0;
    uint64_t lastReportedTracking = 0;

    while (g_running) {
        AlvrEvent event{};
        if (!alvr_poll_event(&event, 100ull * 1000 * 1000)) {
            continue;
        }
        switch (event.tag) {
            case ALVR_EVENT_CLIENT_CONNECTED:
                sawClientConnected = true;
                std::printf("[smoke] >>> ClientConnected\n");
                break;
            case ALVR_EVENT_CLIENT_DISCONNECTED:
                std::printf("[smoke] >>> ClientDisconnected\n");
                break;
            case ALVR_EVENT_BATTERY:
                std::printf("[smoke] Battery: device=%llu gauge=%.2f plugged=%d\n",
                            (unsigned long long)event.battery.info.device_id,
                            event.battery.info.gauge_value, event.battery.info.is_plugged);
                break;
            case ALVR_EVENT_PLAYSPACE_SYNC:
                std::printf("[smoke] PlayspaceSync: %.2f x %.2f\n",
                            event.playspace_sync.bounds[0], event.playspace_sync.bounds[1]);
                break;
            case ALVR_EVENT_VIEWS_CONFIG:
                std::printf("[smoke] ViewsConfig: fov0=(%.2f,%.2f,%.2f,%.2f) ipd~=%.4f\n",
                            event.views_config.fov[0].left, event.views_config.fov[0].right,
                            event.views_config.fov[0].up, event.views_config.fov[0].down,
                            event.views_config.local_view_transform[1].position[0] -
                                event.views_config.local_view_transform[0].position[0]);
                break;
            case ALVR_EVENT_TRACKING_UPDATED: {
                sawTracking = true;
                ++trackingCount;
                const uint64_t ts = event.tracking_updated.sample_timestamp_ns;
                if (trackingCount - lastReportedTracking >= 72) { // ~1/s at 72Hz
                    lastReportedTracking = trackingCount;
                    AlvrDeviceMotion head{};
                    const uint64_t headId = alvr_path_to_id("/user/head");
                    const bool ok = alvr_get_device_motion(headId, ts, &head);
                    std::printf(
                        "[smoke] TrackingUpdated #%llu ts=%llu head_ok=%d pos=(%.2f,%.2f,%.2f)\n",
                        (unsigned long long)trackingCount, (unsigned long long)ts, ok,
                        head.pose.position[0], head.pose.position[1], head.pose.position[2]);
                }
                break;
            }
            case ALVR_EVENT_BUTTONS_UPDATED: {
                const uint64_t count = alvr_get_buttons(nullptr);
                std::vector<AlvrButtonEntry> entries(count);
                if (count > 0) {
                    alvr_get_buttons(entries.data());
                }
                std::printf("[smoke] ButtonsUpdated: %llu entries\n", (unsigned long long)count);
                break;
            }
            case ALVR_EVENT_REQUEST_IDR:
                std::printf("[smoke] RequestIDR (no video path in smoke test)\n");
                break;
            case ALVR_EVENT_CAPTURE_FRAME:
                std::printf("[smoke] CaptureFrame\n");
                break;
            case ALVR_EVENT_RESTART_PENDING:
                std::printf("[smoke] RestartPending\n");
                break;
            case ALVR_EVENT_SHUTDOWN_PENDING:
                std::printf("[smoke] ShutdownPending\n");
                g_running = false;
                break;
            default:
                std::printf("[smoke] unknown event tag %u\n", event.tag);
                break;
        }
    }

    std::printf("[smoke] shutting down (ClientConnected=%d TrackingUpdated=%d count=%llu)\n",
                sawClientConnected, sawTracking, (unsigned long long)trackingCount);
    alvr_shutdown();

    return (sawClientConnected && sawTracking) ? 0 : 1;
}
