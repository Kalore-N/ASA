package com.asa.chat

import android.content.Context
import android.media.AudioAttributes
import android.media.AudioFormat
import android.media.AudioManager
import android.media.AudioRecord
import android.media.AudioTrack
import android.media.MediaPlayer
import android.media.MediaRecorder
import android.media.audiofx.AcousticEchoCanceler
import android.util.Log
import java.io.File
import java.io.RandomAccessFile
import kotlin.math.max

/**
 * Android 端 PCM 音频：16kHz、单声道、16-bit、20ms/帧。
 *
 * 实时通话和语音消息都使用 AudioRecord/AudioTrack，不依赖 ffmpeg。
 */
object VoiceCallManager {
    private const val TAG = "AsaVoice"
    const val SAMPLE_RATE = 16_000
    const val FRAME_BYTES = 640

    private var record: AudioRecord? = null
    private var track: AudioTrack? = null
    private var captureThread: Thread? = null
    private var echo: AcousticEchoCanceler? = null
    private var player: MediaPlayer? = null

    private var messageRecord: AudioRecord? = null
    private var messageThread: Thread? = null
    private var messageFile: RandomAccessFile? = null
    private var messagePath: File? = null
    private var messageBytes = 0L

    @Volatile
    private var running = false

    fun start(context: Context, onFrame: (ByteArray) -> Unit) {
        stop()
        val min = AudioRecord.getMinBufferSize(
            SAMPLE_RATE,
            AudioFormat.CHANNEL_IN_MONO,
            AudioFormat.ENCODING_PCM_16BIT,
        )
        val rec = AudioRecord(
            MediaRecorder.AudioSource.VOICE_COMMUNICATION,
            SAMPLE_RATE,
            AudioFormat.CHANNEL_IN_MONO,
            AudioFormat.ENCODING_PCM_16BIT,
            max(min, FRAME_BYTES * 4),
        )
        if (rec.state != AudioRecord.STATE_INITIALIZED) {
            rec.release()
            error("麦克风初始化失败")
        }
        echo = runCatching {
            AcousticEchoCanceler.create(rec.audioSessionId)?.apply { enabled = true }
        }.getOrNull()
        rec.startRecording()
        record = rec

        val minTrack = AudioTrack.getMinBufferSize(
            SAMPLE_RATE,
            AudioFormat.CHANNEL_OUT_MONO,
            AudioFormat.ENCODING_PCM_16BIT,
        )
        val out = AudioTrack.Builder()
            .setAudioAttributes(
                AudioAttributes.Builder()
                    .setUsage(AudioAttributes.USAGE_VOICE_COMMUNICATION)
                    .setContentType(AudioAttributes.CONTENT_TYPE_SPEECH)
                    .build(),
            )
            .setAudioFormat(
                AudioFormat.Builder()
                    .setEncoding(AudioFormat.ENCODING_PCM_16BIT)
                    .setSampleRate(SAMPLE_RATE)
                    .setChannelMask(AudioFormat.CHANNEL_OUT_MONO)
                    .build(),
            )
            .setBufferSizeInBytes(max(minTrack, FRAME_BYTES * 4))
            .setTransferMode(AudioTrack.MODE_STREAM)
            .build()
        out.play()
        track = out
        setAudioMode(context, true)

        running = true
        captureThread = Thread {
            val buffer = ByteArray(FRAME_BYTES)
            while (running) {
                val read = rec.read(buffer, 0, buffer.size)
                if (read > 0) {
                    onFrame(buffer.copyOf(read))
                } else if (read < 0) {
                    break
                }
            }
        }.apply {
            name = "asa-voice-capture"
            start()
        }
        Log.i(TAG, "voice started")
    }

    fun stop(context: Context? = null) {
        running = false
        captureThread?.let { runCatching { it.join(500) } }
        captureThread = null
        runCatching { record?.stop() }
        runCatching { record?.release() }
        record = null
        runCatching { echo?.release() }
        echo = null
        runCatching { track?.stop() }
        runCatching { track?.release() }
        track = null
        if (context != null) setAudioMode(context, false)
        Log.i(TAG, "voice stopped")
    }

    fun isRunning(): Boolean = running

    fun playFrame(data: ByteArray) {
        if (data.isEmpty()) return
        runCatching { track?.write(data, 0, data.size) }
    }

    fun setSpeaker(context: Context, on: Boolean) {
        val audio = context.getSystemService(AudioManager::class.java) ?: return
        audio.isSpeakerphoneOn = on
    }

    fun setAudioMode(context: Context, active: Boolean) {
        val audio = context.getSystemService(AudioManager::class.java) ?: return
        runCatching {
            audio.mode = if (active) AudioManager.MODE_IN_COMMUNICATION else AudioManager.MODE_NORMAL
        }
    }

    /** 开始录制 WAV 语音消息。 */
    fun startMessage(context: Context, path: File) {
        stopMessage()
        path.parentFile?.mkdirs()
        val file = RandomAccessFile(path, "rw")
        file.setLength(0)
        file.write(ByteArray(44))
        val min = AudioRecord.getMinBufferSize(
            SAMPLE_RATE,
            AudioFormat.CHANNEL_IN_MONO,
            AudioFormat.ENCODING_PCM_16BIT,
        )
        val rec = AudioRecord(
            MediaRecorder.AudioSource.MIC,
            SAMPLE_RATE,
            AudioFormat.CHANNEL_IN_MONO,
            AudioFormat.ENCODING_PCM_16BIT,
            max(min, FRAME_BYTES * 4),
        )
        if (rec.state != AudioRecord.STATE_INITIALIZED) {
            rec.release()
            file.close()
            error("麦克风初始化失败")
        }
        rec.startRecording()
        messageFile = file
        messagePath = path
        messageRecord = rec
        messageBytes = 0
        running = true
        messageThread = Thread {
            val buffer = ByteArray(FRAME_BYTES)
            while (running) {
                val read = rec.read(buffer, 0, buffer.size)
                if (read > 0) {
                    runCatching {
                        messageFile?.write(buffer, 0, read)
                        messageBytes += read
                    }
                } else if (read < 0) {
                    break
                }
            }
        }.apply {
            name = "asa-voice-message"
            start()
        }
        Log.i(TAG, "message recording: ${path.absolutePath}")
    }

    fun stopMessage(): String? {
        running = false
        messageThread?.let { runCatching { it.join(800) } }
        messageThread = null
        runCatching { messageRecord?.stop() }
        runCatching { messageRecord?.release() }
        messageRecord = null
        val file = messageFile
        messageFile = null
        val path = messagePath
        messagePath = null
        if (file == null) return null
        runCatching {
            val dataSize = messageBytes.coerceAtMost(Int.MAX_VALUE.toLong()).toInt()
            val header = wavHeader(dataSize)
            file.seek(0)
            file.write(header)
            file.close()
        }
        Log.i(TAG, "message saved: ${path?.absolutePath}")
        return path?.absolutePath
    }

    private fun wavHeader(dataSize: Int): ByteArray {
        val byteRate = SAMPLE_RATE * 2
        val header = ByteArray(44)
        fun putString(offset: Int, value: String) {
            value.toByteArray(Charsets.US_ASCII).copyInto(header, offset)
        }
        fun putInt(offset: Int, value: Int) {
            header[offset] = (value and 0xff).toByte()
            header[offset + 1] = ((value shr 8) and 0xff).toByte()
            header[offset + 2] = ((value shr 16) and 0xff).toByte()
            header[offset + 3] = ((value shr 24) and 0xff).toByte()
        }
        fun putShort(offset: Int, value: Int) {
            header[offset] = (value and 0xff).toByte()
            header[offset + 1] = ((value shr 8) and 0xff).toByte()
        }
        putString(0, "RIFF")
        putInt(4, 36 + dataSize)
        putString(8, "WAVE")
        putString(12, "fmt ")
        putInt(16, 16)
        putShort(20, 1)
        putShort(22, 1)
        putInt(24, SAMPLE_RATE)
        putInt(28, byteRate)
        putShort(32, 2)
        putShort(34, 16)
        putString(36, "data")
        putInt(40, dataSize)
        return header
    }

    fun playFile(path: String, onDone: (() -> Unit)? = null): Boolean {
        stopFile()
        return runCatching {
            player = MediaPlayer().apply {
                setDataSource(path)
                setOnCompletionListener {
                    it.release()
                    if (player === it) player = null
                    onDone?.invoke()
                }
                prepare()
                start()
            }
            true
        }.getOrElse {
            Log.w(TAG, "play file failed", it)
            onDone?.invoke()
            false
        }
    }

    fun stopFile() {
        runCatching { player?.release() }
        player = null
    }
}
