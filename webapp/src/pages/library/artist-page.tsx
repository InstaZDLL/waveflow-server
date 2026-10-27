import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";

import type { Album, ArtistEnrichment } from "../../api";
import {
  artistQuery,
  artistTopSongsQuery,
  enrichmentRetryDelay,
} from "../../api/queries";
import { ArtistImageControls } from "../../components/artist/artist-image-controls";
import { ArtistSplit } from "../../components/artist/artist-split";
import { Artwork } from "../../components/artwork/artwork";
import { AlbumGrid } from "../../components/catalog/album-grid";
import { SongTable } from "../../components/catalog/song-table";
import { Loading } from "../../components/layout/page-shell";
import { isSplitCandidate } from "../../lib/credit-fragments";
import {
  type DiscographySection,
  discographySections,
} from "../../lib/discography";
import { type TranslationKey, useI18n } from "../../state/i18n";
import { mayCorrectTracks, useLibraryScope } from "../../state/library-scope";

/** A provider as a reader knows it, rather than as the setting spells it. */
const PROVIDER_NAMES: Record<string, string> = {
  lastfm: "Last.fm",
  deezer: "Deezer",
  theaudiodb: "TheAudioDB",
};

/**
 * What the server learned about an artist, and where each part came from.
 *
 * Provenance is part of the data (RFC-011 decision 11): each field names its
 * provider and the day it answered, because the fields are merged across
 * providers and one credit for the whole section would be wrong as soon as
 * two contributed. `none` and `disabled` render nothing; `pending` says so
 * only while the page is still asking.
 */
function ArtistEnrichmentSection({
  enrichment,
  asking,
}: {
  enrichment: ArtistEnrichment | undefined;
  asking: boolean;
}) {
  const { t, locale } = useI18n();
  if (!enrichment) return null;
  const credit = (source: { provider: string; fetched_at: number }) => ({
    provider: PROVIDER_NAMES[source.provider] ?? source.provider,
    date: new Intl.DateTimeFormat(locale, { dateStyle: "medium" }).format(
      new Date(source.fetched_at),
    ),
  });
  const { bio, similar } = enrichment;
  const full = bio?.full && bio.full !== bio.summary ? bio.full : null;
  return (
    <>
      {enrichment.state === "pending" && asking ? (
        <p className="muted artist-enrichment-pending" role="status">
          {t("artists.enrichmentPending")}
        </p>
      ) : null}
      {bio && (bio.summary || full) ? (
        <section className="artist-about">
          <h3>{t("artists.about")}</h3>
          {bio.summary ? <p className="artist-bio">{bio.summary}</p> : null}
          {full ? (
            <details>
              <summary>{t("artists.readMore")}</summary>
              <p className="artist-bio">{full}</p>
            </details>
          ) : null}
          <p className="muted artist-source">
            {t("artists.bioSource", credit(bio.source))}
          </p>
        </section>
      ) : null}
      {similar && similar.artists.length > 0 ? (
        <section className="artist-similar">
          <h3>{t("artists.similar")}</h3>
          <ul className="list artist-list">
            {similar.artists.map((artist) => (
              <li key={artist.id}>
                <Link to="/artists/$artistId" params={{ artistId: artist.id }}>
                  <Artwork
                    artworkId={artist.artwork_hash}
                    title={artist.name}
                    className="artist-avatar"
                  />
                  <strong>{artist.name}</strong>
                </Link>
              </li>
            ))}
          </ul>
          <p className="muted artist-source">
            {t("artists.similarSource", credit(similar.source))}
          </p>
        </section>
      ) : null}
    </>
  );
}

export function ArtistPage({ artistId }: { artistId: string }) {
  const { t, locale } = useI18n();
  const client = useQueryClient();
  const { libraries } = useLibraryScope();
  const query = artistQuery(artistId);
  const { data: value, error } = useQuery(query);
  /**
   * Asked beside the artist rather than with it, and never blocking it.
   *
   * The albums are what the page is; the most played list is what this account
   * made of them. A desktop client fills this part of an artist page from a
   * remote service, which this server queries none of — so what is counted
   * here is local listening, and nothing else.
   */
  const { data: played } = useQuery(artistTopSongsQuery(artistId));
  if (!value) return <Loading error={error} />;
  const image = value.enrichment?.image ?? null;
  // The artist row carries the portrait once the server has linked it; the
  // enrichment names it as soon as it is cached, which may be a visit sooner.
  const portrait = value.artwork_hash ?? image?.artwork_hash ?? null;
  // The banner chosen by hand (RFC-013) or TheAudioDB's fanart (RFC-014), or
  // else the portrait, blurred, as the desktop draws it: a header that carries
  // the artist even with no banner.
  // The enrichment names an automatic banner as soon as it is cached, which
  // may be a visit before the artist row is linked again — as the portrait.
  const bannerSource = value.enrichment?.banner ?? null;
  const banner = value.banner_hash ?? bannerSource?.artwork_hash ?? null;
  const backdrop = banner ?? portrait;
  // Offered where the server would take it: owners and managers.
  const mayChoose = mayCorrectTracks(
    libraries.find((library) => library.id === value.library_id),
  );
  // Counted exactly as `artistQuery` counts, so the note goes when the asking
  // does.
  const asked = client.getQueryState(query.queryKey);
  const asking =
    enrichmentRetryDelay(
      value,
      (asked?.dataUpdateCount ?? 0) + (asked?.errorUpdateCount ?? 0),
    ) !== false;
  return (
    <section>
      <header
        className={`artist-hero${backdrop ? " has-backdrop" : ""}${banner ? " has-banner" : ""}`}
      >
        {backdrop ? (
          <Artwork
            artworkId={backdrop}
            title={value.name}
            className={banner ? "artist-banner" : "artist-hero-ambient"}
          />
        ) : null}
        <Artwork
          artworkId={portrait}
          title={value.name}
          className="artist-portrait"
        />
        <div>
          <span className="eyebrow">{t("common.artist")}</span>
          <h2>{value.name}</h2>
          <p className="muted">
            {t("artists.libraryCount", { count: value.album_count })}
          </p>
          {/* A joined credit, such as "Ashreveal, Ashish Swargiary" (RFC-015):
              offered where the server would list it, to whoever may decide. */}
          {mayChoose && isSplitCandidate(value.name) ? (
            <ArtistSplit artist={value} />
          ) : null}
          {value.image_choice?.portrait ? (
            <p className="muted artist-source">{t("artistImage.chosen")}</p>
          ) : image && image.artwork_hash === portrait ? (
            <p className="muted artist-source">
              {t("artists.portraitSource", {
                provider:
                  PROVIDER_NAMES[image.source.provider] ??
                  image.source.provider,
                date: new Intl.DateTimeFormat(locale, {
                  dateStyle: "medium",
                }).format(new Date(image.source.fetched_at)),
              })}
            </p>
          ) : null}
          {/* Credited only while it is the banner shown: a chosen one came
              from no provider. TheAudioDB's terms ask for the credit. */}
          {bannerSource &&
          !value.image_choice?.banner &&
          bannerSource.artwork_hash === banner ? (
            <p className="muted artist-source">
              {t("artists.bannerSource", {
                provider:
                  PROVIDER_NAMES[bannerSource.source.provider] ??
                  bannerSource.source.provider,
                date: new Intl.DateTimeFormat(locale, {
                  dateStyle: "medium",
                }).format(new Date(bannerSource.source.fetched_at)),
              })}
            </p>
          ) : null}
        </div>
        {mayChoose ? <ArtistImageControls artist={value} /> : null}
      </header>
      <ArtistEnrichmentSection enrichment={value.enrichment} asking={asking} />
      {/* Shown only once there is a history to show. An artist nobody has
          played has nothing to say here, and a heading above an empty table
          would say it anyway. */}
      {played && played.length > 0 ? (
        <section className="artist-played">
          <h3>{t("artists.mostPlayed")}</h3>
          <SongTable songs={played} />
        </section>
      ) : null}
      <ArtistDiscography albums={value.albums} appearsOn={value.appears_on} />
    </section>
  );
}

const SECTION_TITLES: Record<DiscographySection, TranslationKey> = {
  album: "artists.section.album",
  ep: "artists.section.ep",
  single: "artists.section.single",
  live: "artists.section.live",
  compilation: "artists.section.compilation",
  appearsOn: "artists.section.appearsOn",
};

/**
 * The albums, in the sections the server filed them under (RFC-022): Albums,
 * EPs, Singles, Live, Compilations, then what the artist appears on. A section
 * with nothing in it is left out rather than drawn empty, and an artist with
 * nothing at all keeps the grid's own "no albums" line.
 */
function ArtistDiscography({
  albums,
  appearsOn,
}: {
  albums: Album[];
  appearsOn: Album[] | undefined;
}) {
  const { t } = useI18n();
  const sections = discographySections(albums, appearsOn ?? []);
  if (sections.length === 0) return <AlbumGrid albums={[]} />;
  return (
    <>
      {sections.map(({ section, albums: listed }) => (
        <section
          key={section}
          className="artist-discography"
          aria-labelledby={`discography-${section}`}
        >
          <h3 id={`discography-${section}`}>{t(SECTION_TITLES[section])}</h3>
          <AlbumGrid albums={listed} />
        </section>
      ))}
    </>
  );
}
