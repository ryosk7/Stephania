/* Narrow libx264 ABI bridge. All returned NAL storage belongs to the
 * encoder and is valid only until the next encoder call. */
#include <stdint.h>
#include <stddef.h>
#include <x264.h>

typedef struct {
    const uint8_t *data;
    int64_t pts;
    int64_t dts;
    int32_t keyframe;
} t3x264_packet;

void *t3x264_open(int width, int height, int fps, int bitrate, int fast) {
    x264_param_t p;
    if (x264_param_default_preset(&p, fast ? "ultrafast" : "veryfast", "zerolatency") < 0)
        return NULL;
    p.i_csp = X264_CSP_I420;
    p.i_width = width;
    p.i_height = height;
    p.i_fps_num = fps;
    p.i_fps_den = 1;
    p.i_timebase_num = 1;
    p.i_timebase_den = 1000000;
    p.b_annexb = 0;
    p.i_keyint_max = fps * 2;
    p.rc.i_rc_method = X264_RC_ABR;
    p.rc.i_bitrate = bitrate;
    p.rc.i_vbv_max_bitrate = bitrate;
    p.rc.i_vbv_buffer_size = bitrate * 2;
    return x264_encoder_open(&p);
}

void t3x264_close(void *handle) {
    x264_encoder_close(handle);
}

int t3x264_headers(void *handle, t3x264_packet *out) {
    x264_nal_t *nals = NULL;
    int count = 0;
    int size = x264_encoder_headers(handle, &nals, &count);
    if (size > 0 && count > 0) out->data = nals[0].p_payload;
    return size;
}

int t3x264_encode(void *handle, const uint8_t *data, int width, int height,
                  int64_t pts, t3x264_packet *out) {
    x264_picture_t input, output;
    x264_picture_init(&input);
    x264_picture_init(&output);
    if (data) {
        input.img.i_csp = X264_CSP_I420;
        input.img.i_plane = 3;
        input.img.i_stride[0] = width;
        input.img.i_stride[1] = input.img.i_stride[2] = width / 2;
        input.img.plane[0] = (uint8_t *)data;
        input.img.plane[1] = (uint8_t *)data + (size_t)width * height;
        input.img.plane[2] = input.img.plane[1] + (size_t)width * height / 4;
        input.i_pts = pts;
    }
    x264_nal_t *nals = NULL;
    int count = 0;
    int size = x264_encoder_encode(handle, &nals, &count, data ? &input : NULL, &output);
    if (size > 0 && count > 0) {
        out->data = nals[0].p_payload;
        out->pts = output.i_pts;
        out->dts = output.i_dts;
        out->keyframe = output.b_keyframe;
    }
    return size;
}

int t3x264_delayed(void *handle) {
    return x264_encoder_delayed_frames(handle);
}

int t3x264_rate_control(void *handle, int bitrate) {
    x264_param_t p;
    x264_encoder_parameters(handle, &p);
    return p.rc.i_rc_method == X264_RC_ABR &&
           p.rc.i_bitrate == bitrate &&
           p.rc.i_vbv_max_bitrate == bitrate &&
           p.rc.i_vbv_buffer_size == bitrate * 2;
}
