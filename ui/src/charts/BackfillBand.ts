import type {
  IPrimitivePaneRenderer,
  IPrimitivePaneView,
  ISeriesPrimitive,
  SeriesAttachedParameter,
  Time,
} from 'lightweight-charts'

type Target = Parameters<IPrimitivePaneRenderer['draw']>[0]

/** A span of bar times, in chart seconds. */
export interface Band {
  from: Time
  to: Time
}

/** Hatched columns behind the bars, where a backfill sent them. */
export class BackfillBand implements ISeriesPrimitive<Time> {
  private bands: Band[] = []
  private host: SeriesAttachedParameter<Time> | null = null
  private readonly view: IPrimitivePaneView

  constructor() {
    const self = this
    const renderer: IPrimitivePaneRenderer = {
      draw() {},
      drawBackground(target: Target) {
        const chart = self.host?.chart
        if (!chart) return
        const scale = chart.timeScale()
        target.useBitmapCoordinateSpace((scope) => {
          const { context, bitmapSize, horizontalPixelRatio: r } = scope
          for (const band of self.bands) {
            const a = scale.timeToCoordinate(band.from)
            const b = scale.timeToCoordinate(band.to)
            if (a === null || b === null) continue
            const x0 = Math.round(Math.min(a, b) * r)
            const x1 = Math.round(Math.max(a, b) * r)
            context.save()
            context.beginPath()
            context.rect(x0, 0, x1 - x0, bitmapSize.height)
            context.clip()
            context.fillStyle = 'rgba(236, 232, 223, 0.025)'
            context.fillRect(x0, 0, x1 - x0, bitmapSize.height)
            context.strokeStyle = 'rgba(236, 232, 223, 0.06)'
            context.lineWidth = 3 * r
            const step = 8 * r
            for (let x = x0 - bitmapSize.height; x < x1; x += step) {
              context.beginPath()
              context.moveTo(x, bitmapSize.height)
              context.lineTo(x + bitmapSize.height, 0)
              context.stroke()
            }
            context.restore()
            context.strokeStyle = '#5e625f'
            context.lineWidth = r
            context.setLineDash([2 * r, 3 * r])
            for (const x of [x0, x1]) {
              context.beginPath()
              context.moveTo(x + 0.5, 0)
              context.lineTo(x + 0.5, bitmapSize.height)
              context.stroke()
            }
            context.setLineDash([])
          }
        })
      },
    }
    this.view = { zOrder: () => 'bottom', renderer: () => renderer }
  }

  attached(param: SeriesAttachedParameter<Time>) {
    this.host = param
  }

  detached() {
    this.host = null
  }

  paneViews() {
    return [this.view]
  }

  set(bands: Band[]) {
    this.bands = bands
    this.host?.requestUpdate()
  }
}
