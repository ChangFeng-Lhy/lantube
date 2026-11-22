<script setup lang="ts">
import {Clock3, Eye, Film, Globe2, Grid2X2, List} from '@lucide/vue'
import {useI18n} from 'vue-i18n'
import {preferences, qualityOptions} from '../preferences'
const {t} = useI18n()

function normalizeBuffers() {
    if (!Number.isFinite(preferences.forwardBufferSeconds)) preferences.forwardBufferSeconds = 12
    if (!Number.isFinite(preferences.backBufferSeconds)) preferences.backBufferSeconds = 4
    preferences.forwardBufferSeconds = Math.min(60, Math.max(4, Math.round(preferences.forwardBufferSeconds / 4) * 4))
    preferences.backBufferSeconds = Math.min(30, Math.max(0, Math.round(preferences.backBufferSeconds / 4) * 4))
}

const qualityLabel = (quality: typeof qualityOptions[number]) => t(`quality${quality === 'original' ? 'Original' : quality === '1080p' ? '1080' : quality === '720p' ? '720' : '480'}`)
</script>

<template>
    <section class="settings-page">
        <h1>{{ t('settings') }}</h1>
        <div class="settings-panel">
            <label class="settings-row">
                <span><Globe2 :size="20" />{{ t('language') }}</span>
                <select v-model="preferences.language" class="select">
                    <option value="zh" lang="zh-CN">简体中文</option>
                    <option value="en" lang="en">English</option>
                </select>
            </label>
            <label class="settings-row">
                <span><component :is="preferences.view === 'grid' ? Grid2X2 : List" :size="20" />{{ t('view') }}</span>
                <select v-model="preferences.view" class="select">
                    <option value="grid">{{ t('grid') }}</option>
                    <option value="list">{{ t('list') }}</option>
                </select>
            </label>
            <label class="settings-row">
                <span><Film :size="20" />{{ t('defaultQuality') }}</span>
                <select v-model="preferences.quality" class="select">
                    <option v-for="quality in qualityOptions" :key="quality" :value="quality">
                        {{ qualityLabel(quality) }}
                    </option>
                </select>
            </label>
            <label class="settings-row">
                <span><Clock3 :size="20" />{{ t('forwardBuffer') }}</span>
                <span class="settings-value">
                    <input v-model.number="preferences.forwardBufferSeconds" class="number-input" type="number"
                        min="4" max="60" step="4" @change="normalizeBuffers">
                    <small>{{ t('bufferSegmentsHint', {count: Math.round(preferences.forwardBufferSeconds / 4)}) }}</small>
                </span>
            </label>
            <label class="settings-row">
                <span><Clock3 :size="20" />{{ t('backBuffer') }}</span>
                <span class="settings-value">
                    <input v-model.number="preferences.backBufferSeconds" class="number-input" type="number"
                        min="0" max="30" step="4" @change="normalizeBuffers">
                    <small>{{ t('bufferSegmentsHint', {count: Math.round(preferences.backBufferSeconds / 4)}) }}</small>
                </span>
            </label>
            <label class="settings-row">
                <span><Eye :size="20" />{{ t('showHidden') }}</span>
                <input v-model="preferences.showHidden" type="checkbox">
            </label>
        </div>
        <p class="settings-help">{{ t('bufferSettingsHelp') }}</p>
    </section>
</template>

<style scoped>
.settings-page { max-width: 780px; }
h1 { margin: 5px 0 23px; font-size: clamp(28px, 4vw, 38px); letter-spacing: -.045em; }
.settings-panel { overflow: hidden; border: 1px solid var(--border); border-radius: var(--radius); background: var(--panel); }
.settings-row { display: flex; align-items: center; justify-content: space-between; gap: 14px; padding: 19px; }
.settings-row + .settings-row { border-top: 1px solid var(--border); }
.settings-row span { display: flex; align-items: center; gap: 12px; font-size: 14px; }
.settings-row svg { color: var(--accent); }
.settings-row input[type="checkbox"] { width: 19px; height: 19px; accent-color: var(--accent); cursor: pointer; }
.settings-value { justify-content: flex-end; }
.settings-value small { color: var(--muted); font-size: 12px; }
.number-input { width: 74px; padding: 7px 9px; border: 1px solid var(--border); border-radius: 8px; background: var(--bg); color: var(--text); font: inherit; }
.settings-help { margin: 14px 2px 0; color: var(--muted); font-size: 13px; line-height: 1.6; }
</style>
