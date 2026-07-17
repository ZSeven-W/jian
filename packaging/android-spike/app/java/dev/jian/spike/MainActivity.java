package dev.jian.spike;

import android.app.Activity;
import android.os.Bundle;
import android.util.Log;
import android.view.Surface;
import android.view.SurfaceHolder;
import android.view.SurfaceView;

/** M4 Task-1 spike shell: hand the Surface to Rust, draw once. */
public class MainActivity extends Activity implements SurfaceHolder.Callback {
    static {
        System.loadLibrary("spike");
    }

    private native int nativeSpike(Surface surface);

    @Override
    protected void onCreate(Bundle savedInstanceState) {
        super.onCreate(savedInstanceState);
        SurfaceView view = new SurfaceView(this);
        view.getHolder().addCallback(this);
        setContentView(view);
    }

    @Override
    public void surfaceCreated(SurfaceHolder holder) {
        int status = nativeSpike(holder.getSurface());
        Log.i("JianSpike", "nativeSpike returned " + status);
    }

    @Override
    public void surfaceChanged(SurfaceHolder holder, int format, int width, int height) {}

    @Override
    public void surfaceDestroyed(SurfaceHolder holder) {}
}
