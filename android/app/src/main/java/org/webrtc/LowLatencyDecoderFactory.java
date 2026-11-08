package org.webrtc;

import android.media.MediaCodec;
import android.media.MediaCodecInfo;
import android.media.MediaCodecList;
import android.media.MediaCrypto;
import android.media.MediaFormat;
import android.os.Build;
import android.os.Bundle;
import android.util.Log;
import android.view.Surface;
import androidx.annotation.Nullable;
import java.io.IOException;
import java.nio.ByteBuffer;

/**
 * H.264 decoder factory that asks the hardware decoder for its low-latency mode.
 *
 * libwebrtc's own decoder (AndroidVideoDecoder) configures MediaCodec without any low-latency
 * hint, so a vendor decoder is free to hold several frames back before it outputs one. This lives
 * in package org.webrtc because AndroidVideoDecoder and the MediaCodec wrapper types are package
 * private. Everything but H.264, and any failure here, falls back to the stock factory.
 */
public class LowLatencyDecoderFactory implements VideoDecoderFactory {
    private static final String TAG = "LowLatencyDecoder";
    private final HardwareVideoDecoderFactory stock;
    private final EglBase.Context sharedContext;

    public LowLatencyDecoderFactory(@Nullable EglBase.Context sharedContext) {
        this.sharedContext = sharedContext;
        this.stock = new HardwareVideoDecoderFactory(sharedContext);
    }

    @Override
    public VideoCodecInfo[] getSupportedCodecs() {
        return stock.getSupportedCodecs();
    }

    @Nullable
    @Override
    public VideoDecoder createDecoder(VideoCodecInfo codecType) {
        try {
            if ("H264".equalsIgnoreCase(codecType.getName())) {
                MediaCodecInfo info = findH264Decoder();
                if (info != null) {
                    MediaCodecInfo.CodecCapabilities caps = info.getCapabilitiesForType(VideoCodecMimeType.H264.mimeType());
                    Integer color = MediaCodecUtils.selectColorFormat(MediaCodecUtils.DECODER_COLOR_FORMATS, caps);
                    if (color != null) {
                        Log.i(TAG, "H.264 decoder " + info.getName() + " in low-latency mode");
                        return new AndroidVideoDecoder(
                                name -> new LowLatencyWrapper(new MediaCodecWrapperFactoryImpl().createByCodecName(name)),
                                info.getName(), VideoCodecMimeType.H264, color, sharedContext);
                    }
                }
            }
        } catch (Throwable t) {
            Log.w(TAG, "falling back to the stock decoder", t);
        }
        return stock.createDecoder(codecType);
    }

    /** A hardware H.264 decoder, one that advertises FEATURE_LowLatency first. */
    @Nullable
    private static MediaCodecInfo findH264Decoder() {
        MediaCodecInfo best = null;
        for (MediaCodecInfo info : new MediaCodecList(MediaCodecList.REGULAR_CODECS).getCodecInfos()) {
            if (info.isEncoder() || !MediaCodecUtils.codecSupportsType(info, VideoCodecMimeType.H264)) continue;
            if (!MediaCodecUtils.isHardwareAccelerated(info)) continue;
            boolean low = false;
            try {
                low = Build.VERSION.SDK_INT >= 30 && info.getCapabilitiesForType(VideoCodecMimeType.H264.mimeType())
                        .isFeatureSupported(MediaCodecInfo.CodecCapabilities.FEATURE_LowLatency);
            } catch (Throwable ignored) {
            }
            if (low) return info;
            if (best == null) best = info;
        }
        return best;
    }

    /** Adds the low-latency hints to the format and passes every call through. */
    private static final class LowLatencyWrapper implements MediaCodecWrapper {
        private final MediaCodecWrapper w;

        LowLatencyWrapper(MediaCodecWrapper w) {
            this.w = w;
        }

        @Override
        public void configure(MediaFormat format, Surface surface, MediaCrypto crypto, int flags) {
            try {
                if (Build.VERSION.SDK_INT >= 30) format.setInteger(MediaFormat.KEY_LOW_LATENCY, 1);
                format.setInteger(MediaFormat.KEY_PRIORITY, 0); // real time
                // Vendor keys (Qualcomm, Exynos, MediaTek); decoders that do not know one ignore it.
                format.setInteger("vendor.qti-ext-dec-low-latency.enable", 1);
                format.setInteger("vendor.rtc-ext-dec-low-latency.enable", 1);
                format.setInteger("vendor.low-latency.enable", 1);
            } catch (Throwable t) {
                Log.w(TAG, "could not set low-latency keys", t);
            }
            w.configure(format, surface, crypto, flags);
        }

        @Override public void start() { w.start(); }
        @Override public void flush() { w.flush(); }
        @Override public void stop() { w.stop(); }
        @Override public void release() { w.release(); }
        @Override public int dequeueInputBuffer(long timeoutUs) { return w.dequeueInputBuffer(timeoutUs); }
        @Override public void queueInputBuffer(int index, int offset, int size, long presentationTimeUs, int flags) {
            w.queueInputBuffer(index, offset, size, presentationTimeUs, flags);
        }
        @Override public int dequeueOutputBuffer(MediaCodec.BufferInfo info, long timeoutUs) { return w.dequeueOutputBuffer(info, timeoutUs); }
        @Override public void releaseOutputBuffer(int index, boolean render) { w.releaseOutputBuffer(index, render); }
        @Override public MediaFormat getInputFormat() { return w.getInputFormat(); }
        @Override public MediaFormat getOutputFormat() { return w.getOutputFormat(); }
        @Override public MediaFormat getOutputFormat(int index) { return w.getOutputFormat(index); }
        @Override public ByteBuffer getInputBuffer(int index) { return w.getInputBuffer(index); }
        @Override public ByteBuffer getOutputBuffer(int index) { return w.getOutputBuffer(index); }
        @Override public Surface createInputSurface() { return w.createInputSurface(); }
        @Override public void setParameters(Bundle params) { w.setParameters(params); }
        @Override public MediaCodecInfo getCodecInfo() { return w.getCodecInfo(); }
    }
}
