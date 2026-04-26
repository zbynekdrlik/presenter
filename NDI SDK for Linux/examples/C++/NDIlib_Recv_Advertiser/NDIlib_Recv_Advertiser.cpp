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
	if (!NDIlib_initialize())
		return 0;

	// We create an unconnected receiver that will be setup for advertising.
	NDIlib_recv_instance_t pNDI_recv = NDIlib_recv_create_v3();
	if (!pNDI_recv)
		return 0;

	// Create an instance of the receiver advertiser
	NDIlib_recv_advertiser_instance_t pNDI_recv_advertiser = NDIlib_recv_advertiser_create();
	if (!pNDI_recv_advertiser) {
		printf("The receiver advertiser failed to create. Please configure the connection to the NDI discovery server.\n");
		NDIlib_recv_destroy(pNDI_recv);
		NDIlib_destroy();
		return 0;
	}

	// Register the receiver with the advertiser
	NDIlib_recv_advertiser_add_receiver(pNDI_recv_advertiser, pNDI_recv, true, true);

	// Run for five minutes.
	using namespace std::chrono;
	for (const auto start = high_resolution_clock::now(); high_resolution_clock::now() - start < minutes(5);) {
		// The descriptors
		NDIlib_video_frame_v2_t video_frame;
		NDIlib_audio_frame_v2_t audio_frame;
		NDIlib_metadata_frame_t metadata_frame;

		switch (NDIlib_recv_capture_v2(pNDI_recv, &video_frame, &audio_frame, &metadata_frame, 1000)) {
			// No data
			case NDIlib_frame_type_none:
				printf("No data received.\n");
				break;

			// Video data
			case NDIlib_frame_type_video:
				printf("Video data received (%dx%d).\n", video_frame.xres, video_frame.yres);
				NDIlib_recv_free_video_v2(pNDI_recv, &video_frame);
				break;

			// Audio data
			case NDIlib_frame_type_audio:
				printf("Audio data received (%d samples).\n", audio_frame.no_samples);
				NDIlib_recv_free_audio_v2(pNDI_recv, &audio_frame);
				break;

			// Metadata
			case NDIlib_frame_type_metadata:
				printf("Received metadata %s\n", metadata_frame.p_data);
				NDIlib_recv_free_metadata(pNDI_recv, &metadata_frame);
				break;

			// There is a status change on the receiver (e.g. new web interface).
			case NDIlib_frame_type_status_change:
				printf("Receiver connection status changed.\n");
				break;

			case NDIlib_frame_type_source_change:
			{
				const char* p_source_name = nullptr;
				if (NDIlib_recv_get_source_name(pNDI_recv, &p_source_name)) {
					// The name of the source could be NULL, which would mean the receiver is set to be
					// connected to nothing.
					if (p_source_name)
						printf("Source name changed: %s\n", p_source_name);
					else
						printf("Not connected to a source\n");
				}

				// Whether the source name has changed or not, the pointer should be set to the name of the
				// current source and will have to be released.
				if (p_source_name)
					NDIlib_recv_free_string(pNDI_recv, p_source_name);
				break;
			}
		}
	}

	// Remove the receiver from the advertiser before destroying it.
	NDIlib_recv_advertiser_del_receiver(pNDI_recv_advertiser, pNDI_recv);

	// Destroy the receiver advertiser.
	NDIlib_recv_advertiser_destroy(pNDI_recv_advertiser);

	// Destroy the receiver.
	NDIlib_recv_destroy(pNDI_recv);

	// Clean up the initialization.
	NDIlib_destroy();
	return 0;
}
