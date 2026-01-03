import { createApp } from 'vue';
import { createPinia } from 'pinia';
import App from './app/App.vue';
import './style.css';

const app = createApp(App);
app.use(createPinia());
app.mount('#app');
