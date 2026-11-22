import {reactive, watch} from 'vue'

export type Quality = 'original' | '1080p' | '720p' | '480p'
export const qualityOptions: Quality[] = ['original', '1080p', '720p', '480p']
export const bufferMegabytesMin = 32
export const bufferMegabytesMax = 1024
export const bufferMegabytesStep = 32
export const defaultBufferMegabytes = 128
export const backBufferSeconds = 8

const profileBitrates: Record<Exclude<Quality, 'original'>, number> = {
    '1080p': 3_000_000,
    '720p': 2_000_000,
    '480p': 1_000_000,
}

export function profileBitrate(quality: Quality): number | undefined {
    return quality === 'original' ? undefined : profileBitrates[quality]
}

export function clampBufferMegabytes(value: number): number {
    const clamped = Math.min(bufferMegabytesMax, Math.max(bufferMegabytesMin, value))
    return Math.round(clamped / bufferMegabytesStep) * bufferMegabytesStep
}

export function bufferTargets(megabytes: number, bitrate: number) {
    const totalBytes = clampBufferMegabytes(megabytes) * 1024 * 1024
    const safeBitrate = Number.isFinite(bitrate) && bitrate > 0 ? bitrate : 8_000_000
    const reservedBackBytes = safeBitrate * backBufferSeconds / 8
    const forwardBytes = Math.max(totalBytes - reservedBackBytes, totalBytes * 0.25)
    return {
        backSeconds: backBufferSeconds,
        forwardBytes,
        forwardSeconds: Math.max(8, forwardBytes * 8 / safeBitrate),
    }
}

export function bufferStorageKey() {
    return 'lantube.bufferMegabytes'
}

function storedQuality(): Quality {
    const value = localStorage.getItem('lantube.quality')
    return qualityOptions.includes(value as Quality) ? value as Quality : 'original'
}

function storedBufferMegabytes(): number {
    const raw = localStorage.getItem(bufferStorageKey())
    if (raw === null) return defaultBufferMegabytes
    const value = Number(raw)
    return Number.isFinite(value) ? clampBufferMegabytes(value) : defaultBufferMegabytes
}

for (const key of ['lantube.forwardBuffer', 'lantube.backBuffer']) {
    try {
        localStorage.removeItem(key)
    } catch {}
}

const storedLanguage = localStorage.getItem('lantube.language')
const storedView = localStorage.getItem('lantube.view')
const storedSort = localStorage.getItem('lantube.sort')

export const preferences = reactive({
    language: (storedLanguage === 'en' || storedLanguage === 'zh'
        ? storedLanguage
        : navigator.language.startsWith('zh') ? 'zh' : 'en') as 'en' | 'zh',
    view: (storedView === 'list' ? 'list' : 'grid') as 'grid' | 'list',
    sort: (storedSort === 'recent' || storedSort === 'size' ? storedSort : 'name') as 'name' | 'recent' | 'size',
    showHidden: localStorage.getItem('lantube.showHidden') === 'true',
    bufferMegabytes: storedBufferMegabytes(),
    quality: storedQuality(),
})

watch(preferences, ({language, view, sort, showHidden, bufferMegabytes, quality}) => {
    localStorage.setItem('lantube.language', language)
    localStorage.setItem('lantube.view', view)
    localStorage.setItem('lantube.sort', sort)
    localStorage.setItem('lantube.showHidden', String(showHidden))
    localStorage.setItem(bufferStorageKey(), String(bufferMegabytes))
    localStorage.setItem('lantube.quality', quality)
})
