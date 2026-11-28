/* Direct libavcodec H.264 decoder for phone camera tracks (no FFmpeg CLI). */
#include <stdint.h>
#include <stdlib.h>
#include <string.h>
#include <libavcodec/avcodec.h>
#include <libavutil/error.h>
#include <libavutil/pixfmt.h>

typedef struct {
    AVCodecContext *ctx;
    AVFrame *frame;
    AVPacket *packet;
} T3Dec;

void t3dec_close(T3Dec *s) {
    if (!s) return;
    av_packet_free(&s->packet);
    av_frame_free(&s->frame);
    avcodec_free_context(&s->ctx);
    free(s);
}

/* max_pixels bounds every picture libavcodec will allocate: an SPS that
 * declares a larger size fails at activation, before any frame buffer or
 * reference picture is allocated (the size check after decoding is too late). */
T3Dec *t3dec_open(int threads, int64_t max_pixels) {
    const AVCodec *codec = avcodec_find_decoder(AV_CODEC_ID_H264);
    if (!codec) return NULL;
    T3Dec *s = calloc(1, sizeof(*s));
    if (!s) return NULL;
    s->ctx = avcodec_alloc_context3(codec);
    s->frame = av_frame_alloc();
    s->packet = av_packet_alloc();
    if (!s->ctx || !s->frame || !s->packet) goto fail;
    s->ctx->thread_count = threads;
    /* Slice threads only: frame threading adds a frame of latency per thread. */
    s->ctx->thread_type = FF_THREAD_SLICE;
    s->ctx->flags |= AV_CODEC_FLAG_LOW_DELAY;
    if (max_pixels <= 0) goto fail;
    s->ctx->max_pixels = max_pixels;
    if (avcodec_open2(s->ctx, codec, NULL) < 0) goto fail;
    return s;
fail:
    t3dec_close(s);
    return NULL;
}

/* One Annex-B access unit. 0 on success, negative AVERROR otherwise. */
int t3dec_send(T3Dec *s, const uint8_t *data, int len, int64_t pts) {
    if (len <= 0) return AVERROR(EINVAL);
    s->packet->data = (uint8_t *)data;
    s->packet->size = len;
    s->packet->pts = pts;
    int err = avcodec_send_packet(s->ctx, s->packet);
    s->packet->data = NULL;
    s->packet->size = 0;
    return err;
}

/* 1 = frame ready (geometry in *w/*h), 0 = needs input, -2 = unsupported
 * pixel format (frame dropped), other negative = decode error. */
int t3dec_receive(T3Dec *s, int *w, int *h, int64_t *pts) {
    av_frame_unref(s->frame);
    int err = avcodec_receive_frame(s->ctx, s->frame);
    if (err == AVERROR(EAGAIN) || err == AVERROR_EOF) return 0;
    if (err < 0) return err;
    if (s->frame->format != AV_PIX_FMT_YUV420P && s->frame->format != AV_PIX_FMT_YUVJ420P) return -2;
    *w = s->frame->width;
    *h = s->frame->height;
    *pts = s->frame->pts;
    return 1;
}

/* Copies the received frame into tightly packed I420 (w*h*3/2 bytes). */
int t3dec_copy_i420(T3Dec *s, uint8_t *out, int capacity) {
    int w = s->frame->width, h = s->frame->height;
    if (w <= 0 || h <= 0 || (w & 1) || (h & 1) || capacity < w * h * 3 / 2) return -1;
    for (int p = 0; p < 3; ++p) {
        int pw = p ? w / 2 : w, ph = p ? h / 2 : h;
        const uint8_t *src = s->frame->data[p];
        int stride = s->frame->linesize[p];
        for (int y = 0; y < ph; ++y) { memcpy(out, src + (size_t)y * stride, pw); out += pw; }
    }
    return w * h * 3 / 2;
}
