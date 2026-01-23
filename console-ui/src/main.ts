import { createApp } from 'vue';
import { createPinia } from 'pinia';
import App from './app/App.vue';
import { i18n } from './i18n';
import './styles/style.css';
import { ensureExternalLinkInterceptor } from './services/opener';
import { IS_MAC_PLATFORM_KEY, isMacPlatform } from './shared/platform';

const app = createApp(App);
app.use(createPinia());
app.use(i18n);
app.provide(IS_MAC_PLATFORM_KEY, isMacPlatform());
app.mount('#app');

ensureExternalLinkInterceptor();
