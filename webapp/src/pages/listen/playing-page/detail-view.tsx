// The detail view: the track, and its words beside it when it has any.

import { useState } from "react";

import type { Song } from "../../api";
import { Artwork } from "../../components/artwork";
import { CanvasStage } from "../../components/canvas";
import { FileQuality } from "../../components/file-quality";
import { useI18n } from "../../state/i18n";
import { type LyricSheet, LyricsStrip, useLyricSheet } from "./lyrics";

/**
 * Whether the detail view gives the words a column of their own, from what it
 * gave them last and what the sheet now says.
 *
 * **Only once there are words to put in it.** A track without lyrics is most of
 * a library, and a second column held open for them would be an empty half of
 * the screen with "no lyrics" written in its corner; the track takes the
 * middle instead. The queue was the other candidate for that column, and was
 * not taken: it has a page and a panel of its own, it is empty as often as the
 * sheet is, and a column that changes subject from one track to the next is
 * harder to read than one that is either the words or absent.
 *
 * **A wait is not an answer.** While the sheet is out the layout keeps the
 * shape it had, so moving from one track with words to the next does not fold
 * the column away and open it again a moment later.
 */
export function detailSplit(
  previous: boolean,
  state: LyricSheet["state"],
): boolean {
  return state === "waiting" ? previous : state === "sheet";
}

/**
 * The track, and its words beside it.
 *
 * On a wide screen, two columns: the artwork, larger than it was, with the
 * names and what the file is under it, and the words in a tall column of their
 * own. Without words, one column in the middle. Narrower, the two stack, cover
 * first, as they always have — the stylesheet decides that, not this.
 */
export function DetailView({ song }: { song: Song }) {
  const { t } = useI18n();
  const lyrics = useLyricSheet(song.id);
  // Adjusted while rendering rather than in an effect, as the artwork does: an
  // effect would paint one frame in the old shape around the new answer.
  const [split, setSplit] = useState(false);
  const next = detailSplit(split, lyrics.state);
  if (next !== split) setSplit(next);

  return (
    <div className={`playing ${next ? "playing-split" : "playing-solo"}`}>
      <div className="playing-track">
        <CanvasStage trackId={song.id}>
          <Artwork
            artworkId={song.artwork_hash}
            title={song.title}
            className="cover large"
          />
        </CanvasStage>
        <div className="playing-detail">
          <span className="eyebrow">{song.album ?? t("common.album")}</span>
          <h2>{song.title}</h2>
          <p className="muted">{song.artist ?? t("common.unknownArtist")}</p>
          {/* Under the names and above the words: what the file is belongs
              with the track and not with the lyrics, and a listener who came
              here to read along should not have to pass it twice. */}
          <FileQuality song={song} />
          {/* Without a column, the strip says here that there is nothing to
              read, or that it is still asking. */}
          {next ? null : <LyricsStrip lyrics={lyrics} />}
        </div>
      </div>
      {next ? (
        <div className="playing-lyrics">
          <LyricsStrip lyrics={lyrics} />
        </div>
      ) : null}
    </div>
  );
}
