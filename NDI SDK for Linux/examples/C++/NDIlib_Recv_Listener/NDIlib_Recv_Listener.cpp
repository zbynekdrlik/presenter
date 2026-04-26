#include <cstdint>
#include <cstdio>
#include <chrono>
#include <Processing.NDI.Lib.h>

#ifdef _WIN32
#ifdef _WIN64
#pragma comment(lib, "Processing.NDI.Lib.x64.lib")
#else // _WIN64
#pragma comment(lib, "Processing.NDI.Lib.x86.lib")
#endif // _WIN64
#endif // _WIN32

int main(int argc, char* argv[])
{
	// Not required, but "correct" (see the SDK documentation).
	if (!NDIlib_initialize()) {
		return 0;
	}

	// Create an instance of the receiver listener.
	NDIlib_recv_listener_instance_t pNDI_recv_listener = NDIlib_recv_listener_create();
	if (!pNDI_recv_listener) {
		printf("The receiver listener failed to create. Please configure the connection to the NDI discovery server.\n");
		NDIlib_destroy();
		return 0;
	}

	// To remember what our last connected state in order to know when the connection state changes.
	bool last_connected = false;

	// Run for five minutes.
	using namespace std::chrono;
	for (const auto start = high_resolution_clock::now(); high_resolution_clock::now() - start < minutes(5);) {
		// Check to see if the listener is currently connected.
		bool curr_connected = NDIlib_recv_listener_is_connected(pNDI_recv_listener);

		// Has the connection state changed?
		if (last_connected != curr_connected) {
			printf("The listener is now %s.\n", curr_connected ? "connected" : "disconnected");
			last_connected = curr_connected;
		}

		if (!NDIlib_recv_listener_wait_for_receivers(pNDI_recv_listener, 1000)) {
			printf("No change to the receivers found.\n");
			continue;
		}

		// Get the updated list of receivers.
		uint32_t num_receivers = 0;
		const NDIlib_receiver_t* p_receivers = NDIlib_recv_listener_get_receivers(pNDI_recv_listener, &num_receivers);

		// Display all of the found receivers.
		printf("Network receivers (%u found).\n", num_receivers);
		for (uint32_t i = 0; i != num_receivers; i++) {
			printf("%u. %s\n", i + 1, p_receivers[i].p_name);
		}
	}

	// Destroy the receiver advertiser.
	NDIlib_recv_listener_destroy(pNDI_recv_listener);

	// Clean up the initialization.
	NDIlib_destroy();
	return 0;
}
