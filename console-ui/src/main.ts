import { createApp } from 'vue';
import { createPinia } from 'pinia';
import App from './App.vue';
import 'markstream-vue/index.css';
import './style.css';

const app = createApp(App);
app.use(createPinia());
app.mount('#app');
