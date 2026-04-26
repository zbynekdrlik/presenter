#include <cstdio>
#include <chrono>
#include <thread>
#include <algorithm>
#include <Processing.NDI.Lib.h>

// This example uses the MiniAudio library which is very cool
/*	Audio playback and capture library. Choice of public domain or MIT-0. See license statements at the end of this file.
	miniaudio - v0.10.16 - 2020-08-14

	David Reid - davidreidsoftware@gmail.com

	Website:       https://miniaud.io
	Documentation: https://miniaud.io/docs
	GitHub:        https://github.com/dr-soft/miniaudio
*/
#define MINIAUDIO_IMPLEMENTATION
#ifdef __APPLE__
#define MA_NO_RUNTIME_LINKING
#endif
#include "../ThirdParty/miniaudio.h"
#undef min

#ifdef _WIN32
#ifdef _WIN64
#pragma comment(lib, "Processing.NDI.Lib.x64.lib")
#else // _WIN64
#pragma comment(lib, "Processing.NDI.Lib.x86.lib")
#endif // _WIN64
#endif // _WIN32

static const int sample_rate = 48000;
static const int no_channels = std::min(16, MA_MAX_CHANNELS);

void data_callback(ma_device* pDevice, void* pOutput, const void* pInput, ma_uint32 frameCount)
{
	// Get the frame-sync	
	NDIlib_framesync_instance_t pNDI_framesync = (NDIlib_framesync_instance_t)pDevice->pUserData;

	// Get audio samples
	NDIlib_audio_frame_v2_t audio_frame;
	NDIlib_framesync_capture_audio(pNDI_framesync, &audio_frame, pDevice->sampleRate, pDevice->playback.channels, frameCount);

	// Fill in the samples
	for (int ch = 0; ch < audio_frame.no_channels; ch++) {
		const float* p_src = (float*)((uint8_t*)audio_frame.p_data + ch * audio_frame.channel_stride_in_bytes);
		      float* p_dst = ch + (float*)pOutput;

		for (int N = frameCount; N; N--, p_src++, p_dst += pDevice->playback.channels)
			*p_dst = *p_src;
	}

	// Release the video. You could keep the frame if you want and release it later.
	NDIlib_framesync_free_audio(pNDI_framesync, &audio_frame);
}

int main(int argc, char* argv[])
{
	// Not required, but "correct" (see the SDK documentation).
	if (!NDIlib_initialize())
		return 0;

	// Create a finder
	NDIlib_find_instance_t pNDI_find = NDIlib_find_create_v2();
	if (!pNDI_find)
		return 0;

	// Wait until there is one source
	uint32_t no_sources = 0;
	const NDIlib_source_t* p_sources = NULL;
	while (!no_sources) {
		// Wait until the sources on the network have changed
		printf("Looking for sources ...\n");
		NDIlib_find_wait_for_sources(pNDI_find, 1000/* One second */);
		p_sources = NDIlib_find_get_current_sources(pNDI_find, &no_sources);
	}

	// We now have at least one source, so we create a receiver to look at it.
	NDIlib_recv_create_v3_t recv_info;
	recv_info.bandwidth = NDIlib_recv_bandwidth_audio_only;
	NDIlib_recv_instance_t pNDI_recv = NDIlib_recv_create_v3();
	if (!pNDI_recv)
		return 0;

	// Connect to our sources
	NDIlib_recv_connect(pNDI_recv, p_sources + 0);

	// We are now going to use a frame-synchronizer to ensure that the audio is dynamically
	// resampled and time-based con
	NDIlib_framesync_instance_t pNDI_framesync = NDIlib_framesync_create(pNDI_recv);

	// Destroy the NDI finder. We needed to have access to the pointers to p_sources[0]
	NDIlib_find_destroy(pNDI_find);

	ma_device_config config = ma_device_config_init(ma_device_type_playback);
	config.playback.format = ma_format_f32; // Set to ma_format_unknown to use the device's native format.
	config.playback.channels = no_channels; // Set to 0 to use the device's native channel count.
	config.sampleRate = sample_rate;        // Set to 0 to use the device's native sample rate.
	config.dataCallback = data_callback;    // This function will be called when miniaudio needs more data.
	config.pUserData = pNDI_framesync;      // Can be accessed from the device object (device.pUserData).

	ma_device device;
	if (ma_device_init(NULL, &config, &device) != MA_SUCCESS) {
		return -1;  // Failed to initialize the device.
	}

	// Start audio playback
	ma_device_start(&device);

	// Wait for some time.
	std::this_thread::sleep_for(std::chrono::minutes(10));

	// Stop playback
	ma_device_uninit(&device);

	// Free the frame-sync
	NDIlib_framesync_destroy(pNDI_framesync);

	// Destroy the receiver
	NDIlib_recv_destroy(pNDI_recv);

	// Not required, but nice
	NDIlib_destroy();

	// Finished
	return 0;
}
