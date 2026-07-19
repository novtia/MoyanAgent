`POST https://ark.ap-southeast.bytepluses.com/api/v3/contents/generations/tasks` [Try](https://api.byteplus.com/api-explorer/?action=CreateContentsGenerationsTasks&groupName=Video%20Generation%20API&serviceCode=ark&version=2024-01-01)

This topic describes the input and output parameters of the API for creating video generation for your reference. The model will generate a video based on the input image and text information. After the generation is completed, you can query tasks by conditions and obtain the generated video.


<span id="hfIqUF5g"></span>
### Model capabilities<mark><sup>new</sup></mark>


* **Dreamina** **Seedance 2.0 series models** <sup>== </sup>**<sup>new</sup>**<sup> ==</sup>  **(audio video / silent video)** 

   * **Multimodal reference\-based video generation** <sup>== </sup>**<sup>new</sup>**<sup> ==</sup>: Input <ins>reference images (0\-9) + videos (0\-3) + audio (0\-3) + text prompt (optional)</ins> to generate one video. Note: You cannot input audio alone. At least 1 reference video or image must be included. Supports generating new videos, editing videos, and extending videos. See [Video generation tutorial](https://docs.byteplus.com/en/docs/ModelArk/2291680) for detailed code samples.

   * **Image to video (first and last frames)**  : Input <ins>first frame image, last frame image and optional text prompt</ins> to generate 1 target video.

   * **Image to video (first frame)**  : Input <ins>first frame image and optional text prompt</ins> to generate 1 target video.

   * **Text to video** : Input <ins>text prompt</ins> to generate 1 target video.

* **Seedance 1.5 Pro (audio video / silent video)** 

   [Image to video \- first and last frames] [Image to video \- first frame] [Text to video]

* **Seedance 1.0 Pro**

   [Image to video \- first and last frames] [Image to video \- first frame] [Text to video]

* **Seedance 1.0 Pro Fast**

   [Image to video \- first frame] [Text to video]


&nbsp;


<Tabs>
<Tab zoneid="caLJZYu0zp" title="Try">
<TabTitle>Try</TabTitle>

[去调试](https://api.byteplus.com/api-explorer/?action=CreateContentsGenerationsTasks&groupName=Video%20Generation%20API&serviceCode=ark&version=2024-01-01)



</Tab>
<Tab zoneid="uEPJfrqGho" title="Quick start">
<TabTitle>Quick start</TabTitle>

<span>![图片](https://portal.volccdn.com/obj/volcfe/cloud-universal-doc/upload_b9c82890e851fc10cc31f48f9065abc6.png) </span> [Playground](https://console.byteplus.com/ark/region:ark+ap-southeast-1/experience/vision) <span>![图片](https://portal.volccdn.com/obj/volcfe/cloud-universal-doc/upload_2abecd05ca2779567c6d32f0ddc7874d.png) </span> [Model list](https://docs.byteplus.com/en/docs/ModelArk/1330310) <span>![图片](https://portal.volccdn.com/obj/volcfe/cloud-universal-doc/upload_a5fdd3028d35cc512a10bd71b982b6eb.png) </span> [Model billing](https://docs.byteplus.com/en/docs/ModelArk/1544106#8f25f772) <span>![图片](https://portal.volccdn.com/obj/volcfe/cloud-universal-doc/upload_afbcf38bdec05c05089d5de5c3fd8fc8.png) </span> [API key](https://console.byteplus.com/ark/region:ark+ap-southeast-1/apiKey?apikey=%7B%7D)

<span>![图片](https://portal.volccdn.com/obj/volcfe/cloud-universal-doc/upload_57d0bca8e0d122ab1191b40101b5df75.png) </span> [API tutorial](https://docs.byteplus.com/en/docs/ModelArk/1366799) <span>![图片](https://portal.volccdn.com/obj/volcfe/cloud-universal-doc/upload_f45b5cd5863d1eed3bc3c81b9af54407.png) </span> [API reference](https://docs.byteplus.com/en/docs/ModelArk/Video_Generation_API) <span>![图片](https://portal.volccdn.com/obj/volcfe/cloud-universal-doc/upload_1609c71a747f84df24be1e6421ce58f0.png) </span> [FAQs](https://docs.byteplus.com/en/docs/ModelArk/1359411) <span>![图片](https://portal.volccdn.com/obj/volcfe/cloud-universal-doc/upload_bef4bc3de3535ee19d0c5d6c37b0ffdd.png) </span> [Model activation](https://console.byteplus.com/ark/region:ark+ap-southeast-1/openManagement?LLM=%7B%7D&tab=ComputerVision)


</Tab>
<Tab zoneid="Td2XdYI7U2" title="Authentication">
<TabTitle>Authentication</TabTitle>

This interface only supports API Key authentication. Please obtain a long\-term API Key on the [API keys](https://console.byteplus.com/ark/region:ark+ap-southeast-1/apiKey?apikey=%7B%7D) page.


</Tab>
</Tabs>


<span id="kM8oKJJH"></span>
## Request parameters

> Go to [Response parameters](https://docs.byteplus.com/en/docs/ModelArk/1520757#Ag40Ad3H)



---



<span id="0j5hYOcF"></span>
### Request body

**model** `string` `Required`

ID of the model you need to call. You can [activate a model service](https://console.byteplus.com/ark/region:ark+ap-southeast-1/openManagement?LLM=%7B%7D&tab=ComputerVision) and [query the model ID](https://docs.byteplus.com/en/docs/ModelArk/1330310).

You can also use an endpoint ID to call a model, querying its rate limits, billing method (prepaid or postpaid), and status, and using its advanced capabilities such as monitoring and security. For more information, refer to [Obtaining an endpoint ID](https://docs.byteplus.com/en/docs/ModelArk/1099522).


---



**content** `object[]` `Required`

The references provided to the model for video generation, supporting text, image, audio, video, and sample task ID.

<div data-tips="true" data-tips-type="warning" data-tips-is-title="true">Note</div>


<div data-tips="true" data-tips-type="warning">Seedance 2.0 series models do not support direct upload of reference images or videos containing real human faces. The following solutions are provided to make it easier for creatives to use portraits. For details, see <a href="https://docs.byteplus.com/en/docs/ModelArk/2291680">Dreamina Seedance 2.0 series tutorial</a>.</div>



* <div data-tips="true" data-tips-type="warning">Supports using original outputs containing human faces from certain models as input assets</div>


* <div data-tips="true" data-tips-type="warning">Supports using preset digital characters as input assets</div>


* <div data-tips="true" data-tips-type="warning">Supports using authorized real\-person assets as input assets</div>



The following combinations are supported:


* Text

* Text (optional) + image

* Text (optional) + video

* Text (optional) + image + audio

* Text (optional) + image + video

* Text (optional) + video + audio

* Text (optional) + image + video + audio

* Sample task ID: A sample video generated using the seedance model. The model can generate a high\-quality official video based on the sample.


<div data-tips="true" data-tips-type="warning" data-tips-is-title="true">Note</div>


<div data-tips="true" data-tips-type="warning">Seedance 2.0 series models do not support direct upload of reference images or videos containing real human faces. To facilitate creators' use of portraits, ModelArk has provided a series of solutions, see the <a href="https://docs.byteplus.com/en/docs/ModelArk/2291680">Seedance 2.0 series tutorial</a> for details.</div>



Information type

**Text** `object`

The input text information for the model to generate a video.


Attributes


---



content. **type** `string` `Required`

The type of the input content. In this case, set the value to `text`.


---



content. **text** `string` `Required`

Text prompt input to the model, describing the expected generated video.

<div data-tips="true" data-tips-type="tip" data-tips-is-title="true">Tip</div>



* <div data-tips="true" data-tips-type="tip">Supported prompt languages: All models support English prompts; Seedance 2.0 series additionally support Japanese, Indonesian, Spanish, and Portuguese.</div>


* <div data-tips="true" data-tips-type="tip">Recommended prompt length: under 1000 words. Lengthy text will lead to scattered information, and the model may ignore details and only focus on key points, resulting in missing elements in the generated video.</div>


* <div data-tips="true" data-tips-type="tip">See <a href="https://docs.byteplus.com/en/docs/ModelArk/2222480">Dreamina Seedance 2.0 series prompt guide</a> for more tips on using prompts.</div>




---



**Image** `object`

The input image information for the model to generate a video.


Attributes


---



content. **type** `string` `Required`

The type of the input content. In this case, set the value to `image_url`. Supports image URL or image Base64 encoding.


---



content. **image_url** `object` `Required`

The input image object for the model.


Attributes

content.image_url. **url** `string` `Required`

Accepts image URL, Base64\-encoded image, or asset ID.


* URL: Enter the public accessible URL of the image.

* Base64 encoding: Convert the local file to a Base64\-encoded string, then submit to the model. Follow the format: `data:image/<image format>;base64,<Base64 encoding>`. Note that `<image format>` must be lowercase, for example `data:image/png;base64,{base64_image}`.

* Asset ID: The URI of the digital charactercter used for video generation. It follows the format `asset://<ASSET_ID>` and can be obtained from the [Elements & Digital Characters Library](https://console.byteplus.com/ark/region:ark+ap-southeast-1/experience/vision?modelId=seedance-2-0-260128&tab=GenVideo).


<div data-tips="true" data-tips-type="tip" data-tips-is-title="true">Tip</div>


<div data-tips="true" data-tips-type="tip">Requirements for uploading a single image</div>



* <div data-tips="true" data-tips-type="tip">Format: .jpeg, .png, .webp, .bmp, .tiff, .gif. In addition, Seedance 1.5 Pro and Seedance 2.0 series also support .heic and .heif.</div>


* <div data-tips="true" data-tips-type="tip">Aspect ratio (width/height): (0.4, 2.5)</div>


* <div data-tips="true" data-tips-type="tip">Width and height (px): (300, 6000)</div>


* <div data-tips="true" data-tips-type="tip">Size:</div>


   * <div data-tips="true" data-tips-type="tip">Single image must be less than 30 MB</div>


   * <div data-tips="true" data-tips-type="tip">Request body size does not exceed 64 MB.</div>


   * <div data-tips="true" data-tips-type="tip">Do not use Base64 encoding for large files.</div>


* <div data-tips="true" data-tips-type="tip">Number of images:</div>


   * <div data-tips="true" data-tips-type="tip">Image to video \- first frame: 1</div>


   * <div data-tips="true" data-tips-type="tip">Image to video \- first and last frames: 2</div>


   * <div data-tips="true" data-tips-type="tip">Seedance 2.0 series multimodal\-reference to video: 1–9 images</div>



&nbsp;

content. **role** `string` `Required under certain conditions`

The location or purpose of the image. Valid values:

<div data-tips="true" data-tips-type="warning" data-tips-is-title="true">Note</div>



* <div data-tips="true" data-tips-type="warning"><strong>Image to video \- first frame,</strong> <strong>image to video \- first and last frames</strong> , <strong>Multimodal reference video generation</strong> (including reference image, video, and audio) are three mutually exclusive scenarios and <strong>cannot be mixed</strong> .</div>


* <div data-tips="true" data-tips-type="warning">For multimodal reference\-based video generation, you can specify reference images as the first and/or last frame in the prompt to indirectly achieve a "first/last frame + multimodal references" effect.</div>


   <div data-tips="true" data-tips-type="warning">If you need to strictly ensure the first and last frames exactly match the specified images, use <strong>image to video \- first and last frames</strong> instead (set role to <code>first_frame</code> /<code>last_frame</code>).   </div>
   



Image to video \- first frame


* **Supported models:**  All models

* **Values** : You need to pass in one **image_url** object, with the role field set to `first_frame` or leave **role** blank.



Image to video \- first and last frames


* **Supported models:**  Seedance 2.0 series, Seedance 1.5 Pro, Seedance 1.0 Pro.

* **Values:**  Two **image_url** objects must be provided, and the role field is required.

   * Role of the first frame: `first_frame`

   * Role of for the last frame: `last_frame`


<div data-tips="true" data-tips-type="tip" data-tips-is-title="true">Tip</div>


<div data-tips="true" data-tips-type="tip">first\-frame and last\-frame images can be the same one.</div>


<div data-tips="true" data-tips-type="tip">If the aspect ratios of the first and last frame images are inconsistent, the first frame image takes precedence, and the last frame image will be automatically cropped to fit.</div>




Image to video \- reference images


* **Supported models:**  Seedance 2.0 series (1\-9 images)

* **Values** : Required. The **role** field for each reference image must be set to `reference_image`




---



**Video** <sup>== </sup>**<sup>new</sup>**<sup> ==</sup> `object`

Reference video provided to the model. Only Seedance 2.0 series supports video input.

ModelArk trusts face\-containing videos generated by Seedance 2.0 series models. You can use the **original face\-containing videos generated by the above models under your account within the past 30 days** as input assets. For details, see [Trusted outputs as input assets](https://docs.byteplus.com/en/docs/ModelArk/2291680).


Attributes

content. **type** `string` `Required`

Type of the input; in this case, set to `video_url`.

Only video URL is supported.


---



content. **video_url** `object` `Required`

The video object provided to the model.


Attributes

content.video_url. **url** `string` `Required`


* URL: The public URL of the video. Only video URLs are supported.

* Asset ID: The URI of the digital character used for video generation. It follows the format `asset://<ASSET_ID>` and can be obtained from the [digital characters library](https://console.byteplus.com/ark/region:ark+ap-southeast-1/experience/vision?modelId=seedance-2-0-260128&tab=GenVideo).


<div data-tips="true" data-tips-type="tip" data-tips-is-title="true">Tip</div>


<div data-tips="true" data-tips-type="tip">Video input requirements</div>



* <div data-tips="true" data-tips-type="tip">Video format: mp4, mov. Supported encoding formats are listed in the table below.</div>


* <div data-tips="true" data-tips-type="tip">Resolution: 480p, 720p, 1080p, 4k</div>


* <div data-tips="true" data-tips-type="tip">Duration: Each video must be between [2, 15] seconds. Up to 3 reference videos can be submitted, and the total duration of all videos must not exceed 15 seconds.</div>


* <div data-tips="true" data-tips-type="tip">Dimensions:</div>


   * <div data-tips="true" data-tips-type="tip">Aspect ratio (width/height): [0.4, 2.5]</div>


   * <div data-tips="true" data-tips-type="tip">Width and height (px): [300, 6000]</div>


   * <div data-tips="true" data-tips-type="tip">Total pixels: [640×640=409600, 3326×2494=8295044], that is, the product of width and height must fall in the range of [409600, 8295044].</div>


* <div data-tips="true" data-tips-type="tip">Size: Each video must not exceed 200 MB.</div>


* <div data-tips="true" data-tips-type="tip">Frame rate (FPS): [24, 60]</div>




|Container Format |Common Extension Name |**MIME** |Supported Encodings |
|---|---|---|---|
|MP4 |.mp4 |video/mp4 |Video: H.264/AVC, H.265/HEVC<br><br>Audio: AAC, MP3 |
|QuickTime |.mov |video/quicktime |Video: H.264/AVC, H.265/HEVC<br><br>Audio: AAC, MP3 |




---



content. **role** `string` `conditionally required`

Position or purpose of the video. Currently only supports `reference_video`.



---



**Audio** <sup>== </sup>**<sup>new</sup>**<sup> ==</sup>`object`

Audio provided to the model. Only Seedance 2.0 series supports audio input.

**Note** : **Audio cannot be input alone** ; at least one reference video or image must be included.


Attributes

content. **type** `string` `Required`

Type of the input; in this case, set to `audio_url`. Supports audio URL or audio Base64 encoded string.


---



content. **audio_url** `object` `Required`

Audio object provided to the model.


Attributes

content.audio_url. **url** `string` `Required`

URL, Base64\-encoded string or asset ID of the audio.


* Audio URL: The public accessible URL of the audio.

* Base64 encoding: Convert the local file to a Base64 encoded string, then submit it to the model. Follow the format: `data:audio/<audio format>;base64,<Base64 encoding>`.

   `<audio format>` must be lowercase, for example, `data:audio/wav;base64,{base64_audio}`.

* Asset ID: The URI of the digital character used for video generation. It follows the format `asset://<ASSET_ID>` and can be obtained from the [digital character library](https://console.byteplus.com/ark/region:ark+ap-southeast-1/experience/vision?modelId=seedance-2-0-260128&tab=GenVideo).


<div data-tips="true" data-tips-type="tip" data-tips-is-title="true">Tip</div>


<div data-tips="true" data-tips-type="tip">Audio input requirements</div>



* <div data-tips="true" data-tips-type="tip">Format: wav, mp3</div>


* <div data-tips="true" data-tips-type="tip">Duration: The length of a single audio [2, 15] (second); up to three reference audio segments can be provided, and the total duration of all audio must not exceed 15 s.</div>


* <div data-tips="true" data-tips-type="tip">Size:</div>


   * <div data-tips="true" data-tips-type="tip">Each audio file must not exceed 15 MB.</div>


   * <div data-tips="true" data-tips-type="tip">The request body size must not exceed 64 MB.</div>


   * <div data-tips="true" data-tips-type="tip">Do not use Base64 encoding for large files.</div>




---



content. **role** `string` `conditionally required`

Position or purpose of the audio. Currently only supports reference_audio: reference audio.



---



**Sample** `object`

Generate an official video based on the sample task ID. This feature is only supported by s **eedance 1.5 pro** . To learn more about how to use the draft feature and review important notes, see [Draft mode](https://docs.byteplus.com/en/docs/ModelArk/2298881#5acd28c8).


Attributes

content. **type** `string` `Required`

Type of input content. In this case, set it to `draft_task`.


---



content. **draft_task** `object` `Required`

Draft task input to the model.


Attributes

content.draft_task. **id** `string` `Required`

Draft task ID. ModelArk will automatically reuse the user inputs used for the draft video ( **model** , content. **text** , content. **image_url, generate_audio, seed, ratio, duration, camera_fixed** ) to generate the final video. Other parameters can be specified manually. If not specified, the default values of this model will be used.

Two steps are required: Step 1, call this API to generate a draft video. Step 2, if you confirm that the draft video meets expectations, you can call this API to generate the final video based on the draft video task ID returned in Step 1. See [Draft mode](https://docs.byteplus.com/en/docs/ModelArk/2298881#5acd28c8) for detailed instructions.





---



**callback_url** `string`

Fill in the callback notification address for the result of this generation task. When the status of the video generation task changes, ModelArk will send a POST request to this address.

The callback request content structure is consistent with the response body of the [Retrieve a video generation task API](https://docs.byteplus.com/en/docs/ModelArk/1521309).

The status returned by the callback can be one of the following values:


* queued: In queue.

* running: Task is running.

* succeeded: Task succeeded. (If sending fails, that is, no successful delivery confirmation is received within 5 seconds, the callback will be retried 3 times.)

* failed: Task failed. (If sending fails, that is, no successful delivery confirmation is received within 5 seconds, the callback will be retried 3 times.)

* expired: Task timed out, that is, the task has been in **running or queued** state for longer than the expiration time. You can set the expiration time via the **execution_expires_after** parameter.



---



**return_last_frame** `Boolean` `Default value: false`


* **true** : Return the last frame image of the generated video. After setting it to `true`, you can get the last frame image of the video via the [Retrieve a video generation task API](https://docs.byteplus.com/en/docs/ModelArk/1521309). The last frame image is in PNG format. Its width and height pixels are consistent with the generated video, and it has no watermark.

   This parameter can be used to generate multiple consecutive videos: use the last frame of the previously generated video as the first frame of the next video task to quickly generate multiple consecutive videos. See [Generate multiple consecutive videos](https://docs.byteplus.com/en/docs/ModelArk/2298881#141cf7fa) for call examples.

* **false** : Do not return the last frame image of the generated video.



---



**service_tier** `string` `Default value: default`

> Modifying the service tier of a submitted task is not supported.

> The Seedance 2.0 series supports only online inference mode and does not support configuring this parameter.


Specify the service tier type for processing this request. Valid values:


* **default** : Online inference mode, with lower RPM and concurrency quotas (see [Model list](https://docs.byteplus.com/en/docs/ModelArk/1330310)), suitable for scenarios with high requirements for inference efficiency.

* **flex** : Offline inference mode, with higher TPD quota (see [Model list](https://docs.byteplus.com/en/docs/ModelArk/1330310)), priced at 50% of online inference, suitable for scenarios with high tolerance for inference latency.



---



**execution_expires_after** `integer` `Default value: 172800`

Task timeout threshold. Specifies the expiration time of the task after submission (unit: second), calculated from the **created at** timestamp. The default value is 172800 seconds, which is 48 hours. Value range: [3600, 259200].

No matter which **service_tier** you use, it is recommended to set an appropriate timeout according to your business scenario. After this time, the task will be automatically terminated and marked as `expired` status.


---



**generate_audio** `boolean` `Default: true`

> Only supported by Seedance 2.0 series and Seedance 1.5 Pro


Controls whether the generated video includes sound synchronized with the footage.


* `true`: The video output by the model includes synchronized audio. The model will automatically generate matching human voice, sound effects and background music based on the text prompt and visual content. It is recommended to put dialogue content in double quotes to optimize the audio generation effect. For example: The man stopped the woman and said: "Remember, you can't point at the moon with your finger in the future."

* `false`: The video output by the model is a silent video.


<div data-tips="true" data-tips-type="warning" data-tips-is-title="true">Note</div>


<div data-tips="true" data-tips-type="warning">All generated videos with audio are mono, regardless of the number of channels of the input audio.</div>



---



**draft** `boolean` `Default: false`

> Only supported by Seedance 1.5 Pro


Controls whether to enable draft mode. See [Draft mode](https://docs.byteplus.com/en/docs/ModelArk/2298881#5acd28c8) for detailed instructions and notes.


* `true`: Enable draft mode, generate a preview video to quickly verify whether the scene structure, shot scheduling, subject motion match the prompt intent as expected. It consumes fewer tokens than normal videos, so the usage cost is lower.

* `false`: Disable draft mode and generate a video normally.


<div data-tips="true" data-tips-type="tip" data-tips-is-title="true">Tip</div>


<div data-tips="true" data-tips-type="tip">After enabling draft mode, the draft video will be generated in 480p resolution (using other resolutions will cause an error). The last frame return function and offline inference function are not supported.</div>



---



**safety_identifier** <sup>== </sup>**<sup>new</sup>**<sup> ==</sup> `string`

Unique identifier of end users, used to help the platform detect users in your application who may violate the ModelArk usage policy. This identifier is an English string, which must be fixed and unique for a single user, and the length cannot exceed 64 characters. It is recommended to pass in a string generated by hashing the username, user ID or email address to avoid leaking user privacy information.


---



**priority** <sup>== </sup>**<sup>new</sup>**<sup> ==</sup> `integer` `Default 0`

> Only supported by Seedance 2.0


Sets the execution priority of the current request and determines its position in the queue. Valid values: **0–9** . A larger value indicates a higher priority.

By default, requests are executed in **FIFO** (First In, First Out) order. After you set a higher priority, the request is inserted before all lower\-priority requests under the same **Endpoint** .

**Example** :

Assume an Endpoint currently has three queued tasks (status=queued), all with the default priority of 0.

Queue: [Task A: priority=0] → [Task B: priority=0] → [Task C: priority=0]

If you submit a new request with priority=5, the request is moved directly to the front of the queue:

Queue: [New request: priority=5] → [Task A: priority=0] → [Task B: priority=0] → [Task C: priority=0]

<div data-tips="true" data-tips-type="tip" data-tips-is-title="true">Tip</div>


<div data-tips="true" data-tips-type="tip">Note</div>



* <div data-tips="true" data-tips-type="tip">Requests with the same priority are still ordered by FIFO.</div>


* <div data-tips="true" data-tips-type="tip">Priority affects only the queue order. It does not interrupt tasks that are already running (status = <code>running</code>).</div>


* <div data-tips="true" data-tips-type="tip">Priority takes effect only within the same Endpoint and does not affect other Endpoints.</div>


* <div data-tips="true" data-tips-type="tip">Offline inference mode (service_tier=flex) does not support priority configuration.</div>




---



<div data-tips="true" data-tips-type="warning" data-tips-is-title="true">Note</div>



* <div data-tips="true" data-tips-type="warning"><strong>For the resolution, ratio, duration, frames, seed, camera_fixed and watermark parameters, the platform has upgraded the parameter input method, as shown in the example below. All models still support the legacy method.</strong></div>


* <div data-tips="true" data-tips-type="warning">Different models may support different parameters and values. See <a href="https://docs.byteplus.com/en/docs/ModelArk/1366799#9fe4cce0">Output video format</a> for details. If the input parameters or options are not supported by the selected model, the parameter will be ignored or an error will be thrown:</div>


   * <div data-tips="true" data-tips-type="warning">New method: Pass parameters directly in the request body. This method uses <strong>strict validation</strong> . If the parameters entered are incorrect, the model will return an error.</div>


   * <div data-tips="true" data-tips-type="warning">Legacy method: Append \-\-[parameters] after the text prompt. This method uses <strong>loose validation</strong> . If parameters are filled incorrectly, the model will automatically use the default value and will not report an error.</div>




**New method (Recommended): Pass the paramters directly** **in the request body**

```JSON
...
   // Specify the aspect ratio of the generated video to 16:9, duration to 5 seconds, resolution to 720p, seed to 11, and include a watermark. The camera is not fixed.
    "model": "seedance-1-5-pro-251215",
    "content": [
        {
            "type": "text",
            "text": "A kitten yawns at the camera"
        }
    ],
    // All parameters must be written in full; abbreviations are not supported
    "resolution": "720p",
    "ratio":"16:9",
    "duration": 5,
    // "frames": 29, Either duration or frames is required
    "seed": 11,
    "camera_fixed": false,
    "watermark": true
...
```




**Legacy method: Append \-\-[parameter] after the text prompt**

```JSON
...
   // Specify the aspect ratio of the generated video to 16:9, duration to 5 seconds, resolution to 720p, seed to 11, and include a watermark. The camera is not fixed.
    "model": "seedance-1-5-pro-251215",
    "content": [
        {
            "type": "text",
            "text": "A kitten yawns at the camera --rs 720p --rt 16:9 --dur 5 --seed 11 --cf false --wm true"
            // "text": "A kitten yawns at the camera --resolution 720p --ratio 16:9 --duration 5 --seed 11 --camerafixed false --watermark true"
        }
    ]
...
```




---



**resolution** `string`

> Default value for Seedance 2.0 series and Seedance 1.5 Pro: `720p`

> Default value for Seedance 1.0 Pro & Pro Fast: `1080p`


Video resolution. Valid values:


* 480p

* 720p

* 1080p: Not supported by Seedance 2.0 Fast and Seedance 2.0 Mini

* 4k: Only supported by Seedance 2.0


<div data-tips="true" data-tips-type="tip" data-tips-is-title="true">Tip</div>



* <div data-tips="true" data-tips-type="tip">Compared with standard 8\-bit video, Seedance 2.0 4k output uses 10\-bit encoding, preserving richer color gradations and smoother tonal transitions. This makes it suitable for professional video production and HDR content.</div>


* <div data-tips="true" data-tips-type="tip">4k videos are encoded in H.265 (HEVC). Some players and browsers may not support direct playback. For details, see <a href="https://docs.byteplus.com/en/docs/ModelArk/2291680#4k_player">4k player compatibility</a>.</div>




---



**ratio** `string`

> Default value for Seedance 2.0 series and Seedance 1.5 Pro: `adaptive`

> Other models: Default value for text to video is `16:9`; default value for image to video is `adaptive`.


Aspect ratio of the generated video. The width and height pixel values corresponding to different aspect ratios are shown in the table below.


* 16:9

* 4:3

* 1:1

* 3:4

* 9:16

* 21:9

* adaptive: Automatically select the most appropriate aspect ratio based on the input (see the following for details)


<div data-tips="true" data-tips-type="warning" data-tips-is-title="true">Note</div>


<div data-tips="true" data-tips-type="warning">Adaptation rules</div>


<div data-tips="true" data-tips-type="warning">When <strong>ratio</strong> is set to <code>adaptive</code>, the model will automatically adapt the aspect ratio according to the generation scenario. The actual aspect ratio of the generated video can be obtained from the ratio parameter returned by the <a href="https://docs.byteplus.com/en/docs/ModelArk/1521309">Retrieve a video generation task API</a>.</div>


<div data-tips="true" data-tips-type="warning"><strong>Supported models:</strong></div>



* <div data-tips="true" data-tips-type="warning">Supported by Seedance 2.0 series and Seedance 1.5 Pro</div>


* <div data-tips="true" data-tips-type="warning">Only supported for image\-to\-video tasks by other models</div>


   <div data-tips="true" data-tips-type="warning"><strong>Value rules:</strong>   </div>
   

* <div data-tips="true" data-tips-type="warning">Text to video: Intelligently select the most appropriate aspect ratio based on the input prompt.</div>


* <div data-tips="true" data-tips-type="warning">First frame / first and last frames to video: Automatically select the closest aspect ratio based on the ratio of the uploaded first frame image.</div>


* <div data-tips="true" data-tips-type="warning">Multimodal reference video generation: Judged based on the intent of the user's prompt. If it is first\-frame video generation/video editing/video extension, select the closest aspect ratio based on the corresponding image/video; otherwise, select the closest aspect ratio based on the first uploaded media file (priority: video \> image).</div>




Width and height pixel values corresponding to different aspect ratios

Note: For image to video tasks, when the selected video aspect ratio is inconsistent with the aspect ratio of your uploaded image, ModelArk will crop your image, and the cropping will be centered. See [Image cropping rules](https://docs.byteplus.com/en/docs/ModelArk/2298881#f76aafc8) for detailed rules.


|Resolution |Aspect ratio |Width and height pixel value<br><br>Seedance 2.0 series |Width and height pixel value<br><br>Seedance 1.5 Pro |Width and height pixel value<br><br>Seedance 1.0 series |
|---|---|---|---|---|
|480p |16:9 |864×496 |864×496 |864×480 |
| |4:3 |752×560 |752×560 |736×544 |
| |1:1 |640×640 |640×640 |640×640 |
| |3:4 |560×752 |560×752 |544×736 |
| |9:16 |496×864 |496×864 |480×864 |
| |21:9 |992×432 |992×432 |960×416 |
|720p |16:9 |1280×720 |1280×720 |1248×704 |
| |4:3 |1112×834 |1112×834 |1120×832 |
| |1:1 |960×960 |960×960 |960×960 |
| |3:4 |834×1112 |834×1112 |832×1120 |
| |9:16 |720×1280 |720×1280 |704×1248 |
| |21:9 |1470×630 |1470×630 |1504×640 |
|1080p<br><br>> Seedance 2.0 Fast and 2.0 Mini are not supported |16:9 |1920×1080 |1920×1080 |1920×1088 |
| |4:3 |1664×1248 |1664×1248 |1664×1248 |
| |1:1 |1440×1440 |1440×1440 |1440×1440 |
| |3:4 |1248×1664 |1248×1664 |1248×1664 |
| |9:16 |1080×1920 |1080×1920 |1088×1920 |
| |21:9 |2206×946 |2206×946 |2176×928 |
|4k<br><br>> Only supported by Seedance 2.0 |16:9 |3840×2160 |\- |\- |
| |4:3 |3326×2494 |\- |\- |
| |1:1 |2880×2880 |\- |\- |
| |3:4 |2494×3326 |\- |\- |
| |9:16 |2160×3840 |\- |\- |
| |21:9 |4398×1886 |\- |\- |




---



**duration** `integer` `Default value: 5`

> You only need to specify either duration or frames, and frames takes precedence over duration. If you want to generate a video with integer seconds, it is recommended to specify duration.


Generated video duration. Only integers are supported. Unit: second.


* Seedance 1.0 Pro, Seedance 1.0 Pro Fast: [2, 12] s.

* Seedance 1.5 Pro: [4,12] or set to `-1`.

* Seedance 2.0 series: [4,15] or set to `-1`.


<div data-tips="true" data-tips-type="warning" data-tips-is-title="true">Note</div>


<div data-tips="true" data-tips-type="warning">Seedance 2.0 series and Seedance 1.5 Pro support two configuration methods:</div>



* <div data-tips="true" data-tips-type="warning">Specify specific duration: Any integer within the valid range is supported.</div>


* <div data-tips="true" data-tips-type="warning">Intelligent specification: Set to <code>-1</code>, which means the model independently selects the appropriate video length (integer seconds) within the valid range. The actual duration of the generated video can be obtained from the <strong>duration</strong> parameter returned by the <a href="https://docs.byteplus.com/en/docs/ModelArk/1521309">Retrieve a video generation task API</a>. Note that video duration is related to billing, please set it carefully.</div>




---



**frames** `Integer`

> Not supported by Seedance 2.0 series and Seedance 1.5 Pro

> You only need to specify either duration or frames, and frames takes precedence over duration. If you want to generate a video with fractional seconds, it is recommended to specify frames.


Frame count of the generated video. By specifying the number of frames, you can flexibly control the length of the generated video and generate videos with fractional seconds.

Due to the value limit of frames, only certain fractional seconds are supported. You need to calculate the closest number of frames according to the formula.


* Calculation formula: Number of frames = duration × frame rate (24).

* Value range: All integer values in the range [29, 289] that fit the format `25 + 4n` are supported, where n is a positive integer.

   Example: If you need to generate a 2.4\-second video, the number of frames = 2.4 × 24 = 57.6. However, since frames value cannot be 57.6, you can only select the closest value. The closest number of frames calculated according to 25 + 4n is 57, and the actually generated video is 57 / 24 = 2.375 seconds.



---



**seed** `integer` `Default value: -1`

> Seedance 2.0 models are not supported.


Seed integer, used to control the randomness of generated content.

Value range: Integer between [\-1, 2^32\-1].

<div data-tips="true" data-tips-type="warning" data-tips-is-title="true">Note</div>



* <div data-tips="true" data-tips-type="warning">For the same request, if the model receives different seed values, such as not specifying a seed value, setting the seed to \-1 (which will be replaced by a random number), or manually changing the seed value, different results will be generated.</div>


* <div data-tips="true" data-tips-type="warning">For the same request, if the model receives the same seed value, similar results will be generated, but complete consistency is not guaranteed.</div>




---



**camera_fixed** `boolean` `Default value: false`

> Not supported in reference\-to\-image scenarios, and not currently supported by Seedance 2.0 series.


Whether to fix the camera. Valid values:


* true: Fix the camera. ModelArk will append the fixed camera instruction to the user's prompt, but the actual result is not guaranteed.

* false: Do not fix the camera.



---



**watermark** `boolean` `Default value: false`

Whether the generated video contains a watermark. Valid values:


* false: The generated video does not contain a watermark.

* true: An `AI Generated` watermark will be displayed in the lower right corner of the generated video.



---



<span id="Ag40Ad3H"></span>
## Response parameters

> Go to [Request parameters](https://docs.byteplus.com/en/docs/ModelArk/1520757#kM8oKJJH)



---



**id** `string`

Video generation task ID. Stored for only 7 days (calculated from the **created at** timestamp), and will be automatically cleared after expiration.


* When `"draft": true` is set, it is the draft video task ID.

* When `"draft": false` is set, it is the normal video task ID.

   The video generation task creation is an asynchronous API. After obtaining the ID, you need to query the status of the video generation task through the [Retrieve a video generation task API](https://docs.byteplus.com/en/docs/ModelArk/1521309). After the task is successful, the `video_url` of the generated video will be output.




