// The Web Audio graph ReplayGain plays through, and the player's teardown.
import {
  type RefObject,
  useCallback,
  useEffect,
  useRef,
  useState,
} from "react";

import { connectWhenRunning, needsWaking } from "../lib/audio-graph";
import { routableThroughWebAudio } from "../lib/replay-gain";
import { useReplayGainPreference } from "../state/preferences";

/** Part of `PlayerProvider`, called in its place among the provider's hooks. */
export function useReplayGainGraph(
  audio: RefObject<HTMLAudioElement | null>,
  preloader: RefObject<HTMLAudioElement | null>,
) {
  const replayGain = useReplayGainPreference();
  const replayGainOn = replayGain.mode !== "off";
  // The Web Audio graph ReplayGain plays through (RFC-022 decision 1): the
  // element, one GainNode, the speakers. Built once and never torn down — an
  // element cannot be detached from its source node — so turning ReplayGain
  // off afterwards sets the gain to one, and a reload restores the plain path.
  const graph = useRef<{ context: AudioContext; gain: GainNode } | null>(null);
  // A context created but not yet running, so the element is not yet claimed.
  // Kept so a later gesture resumes this one rather than making another.
  const candidate = useRef<AudioContext | null>(null);
  const [gainNode, setGainNode] = useState<GainNode | null>(null);
  // Whether the provider is mounted, for a graph whose wiring finishes after
  // a resume: signing out unmounts the player, possibly while one is pending.
  const graphMounted = useRef(false);

  // Signing out unmounts the player, and an open AudioContext holds the audio
  // device until it is closed — browsers cap how many a page may have. Both
  // the candidate and the graph are closed on the way out. The marker is set
  // again on every mount, because StrictMode and Fast Refresh replay effects
  // on an instance that stays mounted.
  useEffect(() => {
    graphMounted.current = true;
    return () => {
      graphMounted.current = false;
      const pending = candidate.current;
      candidate.current = null;
      if (pending) void pending.close().catch(() => undefined);
      const active = graph.current;
      if (active) {
        graph.current = null;
        setGainNode(null);
        void active.context.close().catch(() => undefined);
      }
      // The element is never in the document, so nothing else stops it: with
      // ReplayGain off it would carry on over the sign-in page. With it on,
      // it stays claimed by the context just closed, and would play as
      // silence on an instance that remounts with its refs kept. Either way
      // it is dropped, and the mount effect makes a fresh one.
      const element = audio.current;
      if (element) {
        element.pause();
        element.removeAttribute("src");
        element.load();
        audio.current = null;
      }
      // The next track's preload never plays, but it holds its source and may
      // still be fetching it for an account that has just left.
      const next = preloader.current;
      if (next) {
        next.removeAttribute("src");
        next.load();
        preloader.current = null;
      }
    };
  }, [audio, preloader]);

  /**
   * Asks a stopped graph to run again. A play that starts by itself — the
   * next track, repeat-one — is not a gesture, and a browser may refuse; but
   * Safari's `interrupted` state, after a call or a timer, resumes this way.
   */
  const wakeGraph = useCallback(() => {
    const context = graph.current?.context;
    if (context && context.state !== "running") {
      void context.resume().catch(() => undefined);
    }
  }, []);

  /**
   * Builds the graph if ReplayGain wants it, and wakes it if it sleeps.
   *
   * Called from the plays a listener starts, which are always a gesture: a
   * browser creates a context suspended outside one, and refuses to resume it
   * there. Never on page load, so a listener who never asked for ReplayGain
   * keeps the element exactly as it was. The element is claimed only once the
   * context runs (`connectWhenRunning`): claimed by a suspended context, it
   * would play as silence.
   */
  const prepareGraph = useCallback(() => {
    const element = audio.current;
    if (!element) return;
    if (graph.current) {
      wakeGraph();
      return;
    }
    if (!replayGainOn) return;
    if (candidate.current) {
      // Still waiting to run: this gesture may be the one it needed.
      void candidate.current.resume().catch(() => undefined);
      return;
    }
    // A cross-origin source would play as silence through the graph. The
    // tickets are relative, so this holds by construction; checked anyway.
    const source = element.currentSrc || element.src;
    if (source && !routableThroughWebAudio(source, window.location.href))
      return;
    const Context =
      window.AudioContext ??
      (window as { webkitAudioContext?: typeof AudioContext })
        .webkitAudioContext;
    if (!Context) return;
    let context: AudioContext;
    try {
      context = new Context();
    } catch {
      return;
    }
    candidate.current = context;
    void connectWhenRunning(context, () => {
      const gain = context.createGain();
      // Last, because it is the step that cannot be undone.
      context
        .createMediaElementSource(element)
        .connect(gain)
        .connect(context.destination);
      return gain;
    }).then((gain) => {
      if (candidate.current === context) candidate.current = null;
      if (!graphMounted.current) {
        // Unmounted while it waited: nobody will ever close it otherwise.
        void context.close().catch(() => undefined);
        return;
      }
      // No graph: the element keeps playing as it did, unlevelled.
      if (!gain) return;
      graph.current = { context, gain };
      setGainNode(gain);
      // Stopped under a playing element — Safari interrupts a context for a
      // call or a timer — it is asked to run again rather than left silent.
      context.addEventListener("statechange", () => {
        if (needsWaking(context.state, audio.current?.paused ?? true))
          wakeGraph();
      });
    });
  }, [replayGainOn, wakeGraph, audio]);

  return { replayGain, replayGainOn, gainNode, wakeGraph, prepareGraph };
}
