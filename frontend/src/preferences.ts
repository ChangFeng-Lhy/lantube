import {reactive, watch} from 'vue'

export type Quality = 'original' | '1080p' | '720p' | '480p'
export const qualityOptions: Quality[] = ['original', '1080p', '720p', '480p']

function storedNumber(key: string, fallback: number, min: number, max: number): number {
    const raw = localStorage.getItem(key)
    if (raw === null) return fallback
    const value = Number(raw)
    return Number.isFinite(value) ? Math.min(max, Math.max(min, Math.round(value))) : fallback
}

function storedQuality(): Quality {
    const value = localStorage.getItem('lantube.quality')
    return qualityOptions.includes(value as Quality) ? value as Quality : 'original'
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
    forwardBufferSeconds: storedNumber('lantube.forwardBuffer', 12, 4, 60),
    backBufferSeconds: storedNumber('lantube.backBuffer', 4, 0, 30),
    quality: storedQuality(),
})

watch(preferences, ({language, view, sort, showHidden, forwardBufferSeconds, backBufferSeconds, quality}) => {
    localStorage.setItem('lantube.language', language)
    localStorage.setItem('lantube.view', view)
    localStorage.setItem('lantube.sort', sort)
    localStorage.setItem('lantube.showHidden', String(showHidden))
    localStorage.setItem('lantube.forwardBuffer', String(forwardBufferSeconds))
    localStorage.setItem('lantube.backBuffer', String(backBufferSeconds))
    localStorage.setItem('lantube.quality', quality)
})
